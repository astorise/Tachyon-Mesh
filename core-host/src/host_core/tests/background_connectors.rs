use super::support_and_cache::*;
use crate::*;

#[tokio::test(flavor = "multi_thread")]
async fn background_scaler_tick_respects_cooldown() {
    use axum::{extract::State, routing::patch, Router};
    use std::sync::Mutex;

    async fn capture_patch(
        State(captured): State<Arc<Mutex<Vec<String>>>>,
        body: Bytes,
    ) -> StatusCode {
        captured
            .lock()
            .expect("captured requests should not be poisoned")
            .push(String::from_utf8_lossy(&body).into_owned());
        StatusCode::OK
    }

    let captured = Arc::new(Mutex::new(Vec::new()));
    let mock_app = Router::new()
        .route(
            "/apis/apps/v1/namespaces/default/deployments/legacy-app",
            patch(capture_patch),
        )
        .with_state(Arc::clone(&captured));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock server should bind");
    let address = listener
        .local_addr()
        .expect("mock server should expose a local address");
    let server = tokio::spawn(async move {
        axum::serve(listener, mock_app)
            .await
            .expect("mock server should stay up");
    });

    std::env::set_var(MOCK_K8S_URL_ENV, format!("http://{address}"));

    let config = autoscaling_test_config(true);
    let concurrency_limits = build_concurrency_limits(&config);
    concurrency_limits
        .get("/api/guest-call-legacy")
        .expect("legacy route should have a limiter")
        .pending_waiters
        .store(75, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || {
        let engine = build_test_metered_engine(&config);
        let mut runner = BackgroundTickRunner::new(
            &engine,
            &config,
            config
                .sealed_route("/system/k8s-scaler")
                .expect("background route should be sealed"),
            "k8s-scaler",
            telemetry::init_test_telemetry(),
            concurrency_limits,
            test_host_identity(35),
            Arc::new(StorageBrokerManager::default()),
            test_route_overrides(),
            test_peer_capabilities(),
            Capabilities::detect(),
            test_host_load(),
        )
        .expect("background scaler should instantiate");

        for _ in 0..7 {
            runner.tick().expect("background tick should succeed");
        }
    })
    .await
    .expect("background runner task should complete");

    std::env::remove_var(MOCK_K8S_URL_ENV);
    server.abort();

    let requests = captured
        .lock()
        .expect("captured requests should not be poisoned");
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|body| body.contains("\"replicas\":2")));
}

#[tokio::test(flavor = "multi_thread")]
async fn background_sqs_connector_dispatches_and_acks_messages() {
    use axum::{extract::State, response::IntoResponse, routing::post, Json, Router};
    use serde_json::{json, Value};
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockQueueState {
        pending: Vec<(String, String)>,
        deleted: Vec<String>,
    }

    async fn receive_messages(
        State(state): State<Arc<Mutex<MockQueueState>>>,
    ) -> impl IntoResponse {
        let state = state
            .lock()
            .expect("mock queue state should not be poisoned");
        Json(json!({
            "messages": state.pending.iter().map(|(body, receipt_handle)| json!({
                "body": body,
                "receipt_handle": receipt_handle,
            })).collect::<Vec<_>>()
        }))
    }

    async fn delete_message(
        State(state): State<Arc<Mutex<MockQueueState>>>,
        body: Bytes,
    ) -> StatusCode {
        let payload: Value = serde_json::from_slice(&body).expect("delete payload should be JSON");
        let receipt_handle = payload["receipt_handle"]
            .as_str()
            .expect("delete payload should include a receipt handle");
        let mut state = state
            .lock()
            .expect("mock queue state should not be poisoned");
        state.deleted.push(receipt_handle.to_owned());
        state
            .pending
            .retain(|(_, pending_receipt)| pending_receipt != receipt_handle);
        StatusCode::OK
    }

    let queue_state = Arc::new(Mutex::new(MockQueueState {
        pending: vec![("hello from queue".to_owned(), "receipt-1".to_owned())],
        deleted: Vec::new(),
    }));
    let queue_app = Router::new()
        .route("/queue/receive", post(receive_messages))
        .route("/queue/delete", post(delete_message))
        .with_state(Arc::clone(&queue_state));
    let queue_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock queue listener should bind");
    let queue_address = queue_listener
        .local_addr()
        .expect("mock queue listener should expose an address");
    let queue_server = tokio::spawn(async move {
        axum::serve(queue_listener, queue_app)
            .await
            .expect("mock queue server should stay up");
    });

    let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("host listener should bind");
    let host_address = host_listener
        .local_addr()
        .expect("host listener should expose an address");
    let config = sqs_connector_test_config(
        host_address,
        format!("http://{queue_address}/queue"),
        "/api/guest-example",
        "guest-example",
    );
    let host_app = build_app(build_test_state(
        config.clone(),
        telemetry::init_test_telemetry(),
    ));
    let host_server = tokio::spawn(async move {
        axum::serve(host_listener, host_app)
            .await
            .expect("host app should stay up");
    });

    tokio::task::spawn_blocking(move || {
        let engine = build_test_metered_engine(&config);
        let mut runner = BackgroundTickRunner::new(
            &engine,
            &config,
            config
                .sealed_route("/system/sqs-connector")
                .expect("connector route should be sealed"),
            "system-faas-sqs",
            telemetry::init_test_telemetry(),
            build_concurrency_limits(&config),
            test_host_identity(36),
            Arc::new(StorageBrokerManager::default()),
            test_route_overrides(),
            test_peer_capabilities(),
            Capabilities::detect(),
            test_host_load(),
        )
        .expect("SQS connector should instantiate");
        runner.tick().expect("SQS connector tick should succeed");
    })
    .await
    .expect("background connector task should complete");

    host_server.abort();
    queue_server.abort();

    let queue_state = queue_state
        .lock()
        .expect("mock queue state should not be poisoned");
    assert_eq!(queue_state.deleted, vec!["receipt-1".to_owned()]);
    assert!(queue_state.pending.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn background_sqs_connector_leaves_failed_messages_unacked() {
    use axum::{extract::State, response::IntoResponse, routing::post, Json, Router};
    use serde_json::{json, Value};
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockQueueState {
        pending: Vec<(String, String)>,
        deleted: Vec<String>,
    }

    async fn receive_messages(
        State(state): State<Arc<Mutex<MockQueueState>>>,
    ) -> impl IntoResponse {
        let state = state
            .lock()
            .expect("mock queue state should not be poisoned");
        Json(json!({
            "messages": state.pending.iter().map(|(body, receipt_handle)| json!({
                "body": body,
                "receipt_handle": receipt_handle,
            })).collect::<Vec<_>>()
        }))
    }

    async fn delete_message(
        State(state): State<Arc<Mutex<MockQueueState>>>,
        body: Bytes,
    ) -> StatusCode {
        let payload: Value = serde_json::from_slice(&body).expect("delete payload should be JSON");
        let receipt_handle = payload["receipt_handle"]
            .as_str()
            .expect("delete payload should include a receipt handle");
        let mut state = state
            .lock()
            .expect("mock queue state should not be poisoned");
        state.deleted.push(receipt_handle.to_owned());
        state
            .pending
            .retain(|(_, pending_receipt)| pending_receipt != receipt_handle);
        StatusCode::OK
    }

    let queue_state = Arc::new(Mutex::new(MockQueueState {
        pending: vec![("force-fail".to_owned(), "receipt-2".to_owned())],
        deleted: Vec::new(),
    }));
    let queue_app = Router::new()
        .route("/queue/receive", post(receive_messages))
        .route("/queue/delete", post(delete_message))
        .with_state(Arc::clone(&queue_state));
    let queue_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock queue listener should bind");
    let queue_address = queue_listener
        .local_addr()
        .expect("mock queue listener should expose an address");
    let queue_server = tokio::spawn(async move {
        axum::serve(queue_listener, queue_app)
            .await
            .expect("mock queue server should stay up");
    });

    let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("host listener should bind");
    let host_address = host_listener
        .local_addr()
        .expect("host listener should expose an address");
    let config = sqs_connector_test_config(
        host_address,
        format!("http://{queue_address}/queue"),
        "/api/connector-target",
        "guest-flaky",
    );
    let host_app = build_app(build_test_state(
        config.clone(),
        telemetry::init_test_telemetry(),
    ));
    let host_server = tokio::spawn(async move {
        axum::serve(host_listener, host_app)
            .await
            .expect("host app should stay up");
    });

    tokio::task::spawn_blocking(move || {
        let engine = build_test_metered_engine(&config);
        let mut runner = BackgroundTickRunner::new(
            &engine,
            &config,
            config
                .sealed_route("/system/sqs-connector")
                .expect("connector route should be sealed"),
            "system-faas-sqs",
            telemetry::init_test_telemetry(),
            build_concurrency_limits(&config),
            test_host_identity(37),
            Arc::new(StorageBrokerManager::default()),
            test_route_overrides(),
            test_peer_capabilities(),
            Capabilities::detect(),
            test_host_load(),
        )
        .expect("SQS connector should instantiate");
        runner.tick().expect("SQS connector tick should succeed");
    })
    .await
    .expect("background connector task should complete");

    host_server.abort();
    queue_server.abort();

    let queue_state = queue_state
        .lock()
        .expect("mock queue state should not be poisoned");
    assert!(queue_state.deleted.is_empty());
    assert_eq!(queue_state.pending.len(), 1);
    assert_eq!(queue_state.pending[0].1, "receipt-2");
}

#[tokio::test(flavor = "multi_thread")]
async fn background_cdc_dispatches_events_and_acks_outbox_rows() {
    let state_dir = unique_test_dir("tachyon-cdc-target");
    fs::create_dir_all(&state_dir).expect("cdc state dir should create");

    let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("host listener should bind");
    let host_address = host_listener
        .local_addr()
        .expect("host listener should expose an address");

    let mut target_route = targeted_route(
        "/api/cdc-target",
        vec![weighted_target("guest-volume", 100)],
    );
    target_route.volumes = vec![mounted_volume(&state_dir, "/app/data")];

    let mut cdc_route = system_targeted_route("/system/cdc", "system-faas-cdc");
    cdc_route.env = route_env(&[
        ("DB_URL", "outbox://integration"),
        ("OUTBOX_TABLE", "events_outbox"),
        ("TARGET_ROUTE", "/api/cdc-target"),
        ("BATCH_SIZE", "4"),
    ]);

    let config = IntegrityConfig {
        host_address: host_address.to_string(),
        routes: vec![target_route, cdc_route],
        ..IntegrityConfig::default_sealed()
    };
    let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
    let event_id = data_events::enqueue_event(
        state.storage_broker.core_store.as_ref(),
        "outbox://integration",
        "events_outbox",
        br#"{"event":"user.created"}"#.to_vec(),
        "application/json",
    )
    .expect("outbox event should enqueue");
    assert_eq!(
        data_events::pending_count(
            state.storage_broker.core_store.as_ref(),
            "outbox://integration",
            "events_outbox",
        )
        .expect("pending count should be readable"),
        1
    );

    let host_app = build_app(state.clone());
    let host_server = tokio::spawn(async move {
        axum::serve(host_listener, host_app)
            .await
            .expect("host app should stay up");
    });

    tokio::task::spawn_blocking({
        let config = config.clone();
        let route_overrides = Arc::clone(&state.route_overrides);
        let peer_capabilities = Arc::clone(&state.peer_capabilities);
        let host_load = Arc::clone(&state.host_load);
        let storage_broker = Arc::clone(&state.storage_broker);
        let host_identity = Arc::clone(&state.host_identity);
        let host_capabilities = state.host_capabilities;
        move || {
            let engine = build_test_metered_engine(&config);
            let mut runner = BackgroundTickRunner::new(
                &engine,
                &config,
                config
                    .sealed_route("/system/cdc")
                    .expect("cdc route should be sealed"),
                "system-faas-cdc",
                telemetry::init_test_telemetry(),
                build_concurrency_limits(&config),
                host_identity,
                storage_broker,
                route_overrides,
                peer_capabilities,
                host_capabilities,
                host_load,
            )
            .expect("cdc component should instantiate");
            runner.tick().expect("cdc tick should succeed");
        }
    })
    .await
    .expect("cdc background task should complete");

    host_server.abort();
    let _ = host_server.await;

    assert_eq!(
        fs::read_to_string(state_dir.join("state.txt"))
            .expect("cdc target route should persist event payload"),
        r#"{"event":"user.created"}"#
    );
    assert_eq!(
        data_events::pending_count(
            state.storage_broker.core_store.as_ref(),
            "outbox://integration",
            "events_outbox",
        )
        .expect("pending count should be readable"),
        0
    );
    assert!(
        data_events::claim_events(
            state.storage_broker.core_store.as_ref(),
            "outbox://integration",
            "events_outbox",
            4,
        )
        .expect("claim should succeed")
        .is_empty(),
        "acknowledged event `{event_id}` should no longer be claimable"
    );

    let _ = fs::remove_dir_all(state_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn s3_proxy_forwards_upload_and_buffers_mesh_event() {
    use axum::{
        body::Bytes as AxumBytes,
        extract::{Path as AxumPath, State},
        response::IntoResponse,
        routing::put,
        Router,
    };
    use serde_json::Value;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MockS3State {
        paths: Vec<String>,
        auth_headers: Vec<String>,
        bodies: Vec<Vec<u8>>,
    }

    async fn put_object(
        AxumPath(key): AxumPath<String>,
        State(state): State<Arc<Mutex<MockS3State>>>,
        headers: HeaderMap,
        body: AxumBytes,
    ) -> impl IntoResponse {
        let mut state = state.lock().expect("mock s3 state should not be poisoned");
        state.paths.push(key);
        state.auth_headers.push(
            headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned(),
        );
        state.bodies.push(body.to_vec());
        StatusCode::OK
    }

    let s3_state = Arc::new(Mutex::new(MockS3State::default()));
    let s3_app = Router::new()
        .route("/bucket/{key}", put(put_object))
        .with_state(Arc::clone(&s3_state));
    let s3_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock s3 listener should bind");
    let s3_address = s3_listener
        .local_addr()
        .expect("mock s3 listener should expose an address");
    let s3_server = tokio::spawn(async move {
        axum::serve(s3_listener, s3_app)
            .await
            .expect("mock s3 server should stay up");
    });

    let queue_dir = unique_test_dir("tachyon-s3-proxy-queue");
    let event_dir = unique_test_dir("tachyon-s3-proxy-events");
    fs::create_dir_all(&queue_dir).expect("buffer queue dir should create");
    fs::create_dir_all(&event_dir).expect("event dir should create");

    let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("host listener should bind");
    let host_address = host_listener
        .local_addr()
        .expect("host listener should expose an address");

    let mut buffer_route = system_targeted_route("/system/buffer", "buffer");
    buffer_route.env = route_env(&[
        ("BUFFER_DIR", "/buffer"),
        ("RAM_QUEUE_CAPACITY", "4"),
        ("REPLAY_CPU_LIMIT", "70"),
        ("REPLAY_RAM_LIMIT", "70"),
        ("REPLAY_BATCH_SIZE", "4"),
    ]);
    buffer_route.volumes = vec![mounted_volume(&queue_dir, "/buffer")];

    let mut event_route = targeted_route(
        "/api/upload-events",
        vec![weighted_target("guest-volume", 100)],
    );
    event_route.volumes = vec![mounted_volume(&event_dir, "/app/data")];

    let mut proxy_route = system_targeted_route("/system/s3-proxy", "system-faas-s3-proxy");
    proxy_route.env = route_env(&[
        (
            "REAL_S3_BUCKET",
            format!("http://{s3_address}/bucket").as_str(),
        ),
        ("TARGET_ROUTE", "/api/upload-events"),
        ("BUFFER_ROUTE", "/system/buffer"),
        ("S3_AUTHORIZATION", "Bearer proxy-secret"),
    ]);

    let config = IntegrityConfig {
        host_address: host_address.to_string(),
        routes: vec![event_route, buffer_route.clone(), proxy_route],
        ..IntegrityConfig::default_sealed()
    };
    let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
    let host_app = build_app(state.clone());
    let host_server = tokio::spawn(async move {
        axum::serve(host_listener, host_app)
            .await
            .expect("host app should stay up");
    });

    let response = Client::new()
        .put(format!("http://{host_address}/system/s3-proxy"))
        .header("content-type", "text/plain")
        .header("x-tachyon-object-key", "demo.txt")
        .body("hello-object")
        .send()
        .await
        .expect("s3 proxy upload should succeed");
    assert_eq!(response.status(), StatusCode::OK);
    let payload: Value = serde_json::from_slice(
        &response
            .bytes()
            .await
            .expect("s3 proxy response body should be readable"),
    )
    .expect("s3 proxy response should be JSON");
    assert_eq!(payload["key"], "demo.txt");
    assert_eq!(payload["size_bytes"], 12);

    tokio::task::spawn_blocking({
        let config = config.clone();
        let route_overrides = Arc::clone(&state.route_overrides);
        let peer_capabilities = Arc::clone(&state.peer_capabilities);
        let host_load = Arc::clone(&state.host_load);
        let storage_broker = Arc::clone(&state.storage_broker);
        let host_identity = Arc::clone(&state.host_identity);
        let host_capabilities = state.host_capabilities;
        move || {
            let engine = build_test_metered_engine(&config);
            let mut runner = BackgroundTickRunner::new(
                &engine,
                &config,
                config
                    .sealed_route("/system/buffer")
                    .expect("buffer route should be sealed"),
                "buffer",
                telemetry::init_test_telemetry(),
                build_concurrency_limits(&config),
                host_identity,
                storage_broker,
                route_overrides,
                peer_capabilities,
                host_capabilities,
                host_load,
            )
            .expect("buffer component should instantiate");
            runner.tick().expect("buffer tick should succeed");
        }
    })
    .await
    .expect("buffer replay task should complete");

    host_server.abort();
    s3_server.abort();
    let _ = host_server.await;
    let _ = s3_server.await;

    let s3_state = s3_state
        .lock()
        .expect("mock s3 state should not be poisoned");
    assert_eq!(s3_state.paths, vec!["demo.txt".to_owned()]);
    assert_eq!(
        s3_state.auth_headers,
        vec!["Bearer proxy-secret".to_owned()]
    );
    assert_eq!(s3_state.bodies, vec![b"hello-object".to_vec()]);
    drop(s3_state);

    let event_payload = fs::read_to_string(event_dir.join("state.txt"))
        .expect("upload event route should persist metadata payload");
    let event: Value =
        serde_json::from_str(&event_payload).expect("event payload should decode as JSON");
    assert_eq!(event["bucket"], format!("http://{s3_address}/bucket"));
    assert_eq!(event["key"], "demo.txt");
    assert_eq!(event["content_type"], "text/plain");
    assert_eq!(event["size_bytes"], 12);

    let _ = fs::remove_dir_all(queue_dir);
    let _ = fs::remove_dir_all(event_dir);
}
