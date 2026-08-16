use super::support_and_cache::*;
use crate::*;

#[tokio::test]
async fn router_sheds_system_routes_when_host_is_saturated() {
    let telemetry = telemetry::init_test_telemetry();
    let mut active_guards = Vec::new();
    for _ in 0..=SYSTEM_ROUTE_ACTIVE_REQUEST_THRESHOLD {
        active_guards.push(telemetry::begin_request(&telemetry));
    }

    let app = build_app(build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry,
    ));

    let response = app
        .oneshot(
            Request::get("/metrics")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    drop(active_guards);
}

#[tokio::test]
async fn router_emits_async_telemetry_metrics() {
    use serde_json::Value;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    let captured = Arc::new(Mutex::new(Vec::new()));
    let telemetry = telemetry::init_test_telemetry_with_emitter({
        let captured = Arc::clone(&captured);
        move |line| {
            captured
                .lock()
                .expect("captured telemetry should not be poisoned")
                .push(line);
            true
        }
    });
    let app = build_app(build_test_state(
        IntegrityConfig {
            telemetry_sample_rate: 1.0,
            ..IntegrityConfig::default_sealed()
        },
        telemetry,
    ));

    let response = app
        .oneshot(
            Request::post("/api/guest-example")
                .body(Body::from("Hello Lean FaaS!"))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::OK);

    let line = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(line) = captured
                .lock()
                .expect("captured telemetry should not be poisoned")
                .first()
                .cloned()
            {
                break line;
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("telemetry line should be emitted");
    let record: Value = serde_json::from_str(&line).expect("telemetry output should be valid JSON");

    assert_eq!(record["path"], "/api/guest-example");
    assert_eq!(record["sampled"], true);
    assert_eq!(record["status"], 200);
    assert!(record["trace_id"].as_str().is_some());
    assert!(record["traceparent"].as_str().is_some());
    assert!(record["fuel_consumed"].as_u64().is_some());
    assert!(record["total_duration_us"].as_u64().is_some());
    assert!(record["wasm_duration_us"].as_u64().is_some());
    assert!(record["host_overhead_us"].as_u64().is_some());
}

#[cfg(feature = "admin-plane")]
#[tokio::test]
async fn admin_metrics_endpoint_returns_runtime_snapshot() {
    let telemetry = telemetry::init_test_telemetry();
    let _guard = telemetry::begin_request(&telemetry);
    let state = build_test_state(IntegrityConfig::default_sealed(), telemetry);

    let response = admin_metrics_handler(State(state)).await.0;

    assert_eq!(response.source, "core-host://runtime-telemetry");
    assert_eq!(response.queue_depth, 1);
}

#[cfg(feature = "admin-plane")]
#[tokio::test]
async fn admin_shadow_diffs_endpoint_returns_json_array() {
    let response = admin_shadow_diffs_handler().await.0;

    assert!(response.is_empty());
}

#[cfg(feature = "admin-plane")]
#[tokio::test]
async fn admin_chaos_endpoint_accepts_supported_scenario() {
    let response = admin_chaos_scenario_handler(axum::Json(AdminChaosScenarioRequest {
        scenario: "cpu_pressure".to_owned(),
        duration_seconds: Some(30),
        target: Some("node-a".to_owned()),
    }))
    .await;

    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn router_skips_telemetry_export_for_unsampled_requests() {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    let captured = Arc::new(Mutex::new(Vec::new()));
    let telemetry = telemetry::init_test_telemetry_with_emitter({
        let captured = Arc::clone(&captured);
        move |line| {
            captured
                .lock()
                .expect("captured telemetry should not be poisoned")
                .push(line);
            true
        }
    });
    let app = build_app(build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry,
    ));

    let response = app
        .oneshot(
            Request::post("/api/guest-example")
                .body(Body::from("Hello Lean FaaS!"))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        captured
            .lock()
            .expect("captured telemetry should not be poisoned")
            .is_empty(),
        "unsampled requests should not enqueue telemetry export records"
    );
}

// Regression test for #400: `faas_handler`'s `active_requests` gauge and
// `TelemetryEvent::RequestEnd` used to be scoped to "the handler returned a
// `Response`," not "the response body finished draining." For a streaming
// response (here: the `x-tachyon-route-override` direct-streaming path
// forwarding to a slow peer) that meant both dropped the instant headers
// were ready, even though the peer kept sending chunks for a while after.
// `TelemetryCompletionBody` fixes this by deferring both to the body's own
// completion. This drives a real streaming response end to end through
// `faas_handler` and asserts `active_requests` stays elevated across two
// separate chunks and only drops — and `RequestEnd` only fires — once the
// peer's stream actually ends.
#[tokio::test]
async fn streaming_override_response_keeps_active_requests_elevated_until_stream_ends() {
    use axum::{body::Body as AxumBody, response::Response as AxumResponse, routing::any, Router};
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    async fn drip_two_chunks_then_end() -> AxumResponse {
        let (sender, receiver) = mpsc::channel::<std::result::Result<Bytes, StreamForwardError>>(1);
        tokio::spawn(async move {
            let _ = sender.send(Ok(Bytes::from_static(b"chunk-1"))).await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            let _ = sender.send(Ok(Bytes::from_static(b"chunk-2"))).await;
            // Dropping `sender` here ends the stream cleanly.
        });
        AxumResponse::builder()
            .status(StatusCode::OK)
            .body(AxumBody::new(TimeoutBoundedStreamBody { receiver }))
            .expect("peer response should build")
    }

    let peer_app = Router::new().route(DEFAULT_ROUTE, any(drip_two_chunks_then_end));
    let peer_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("peer listener should bind");
    let peer_address = peer_listener
        .local_addr()
        .expect("peer listener should expose an address");
    let peer_server = tokio::spawn(async move {
        axum::serve(peer_listener, peer_app)
            .await
            .expect("peer app should stay up");
    });

    let captured = Arc::new(Mutex::new(Vec::new()));
    let telemetry = telemetry::init_test_telemetry_with_emitter({
        let captured = Arc::clone(&captured);
        move |line| {
            captured
                .lock()
                .expect("captured telemetry should not be poisoned")
                .push(line);
            true
        }
    });
    let telemetry_handle = telemetry.clone();

    let config = validate_integrity_config(IntegrityConfig {
        telemetry_sample_rate: 1.0,
        ..IntegrityConfig::default_sealed()
    })
    .expect("config should validate");
    let state = build_test_state(config, telemetry);
    update_control_plane_route_override(
        state.route_overrides.as_ref(),
        &state.peer_capabilities,
        DEFAULT_ROUTE,
        &format!("http://{peer_address}{DEFAULT_ROUTE}"),
    )
    .expect("route override should install");

    assert_eq!(telemetry::active_requests(&telemetry_handle), 0);

    let app = build_app(state);
    let response = app
        .oneshot(
            Request::get(DEFAULT_ROUTE)
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::OK);

    // Headers are ready, but the peer is still mid-stream: the old behavior
    // would already have dropped the slot here.
    assert_eq!(
        telemetry::active_requests(&telemetry_handle),
        1,
        "active_requests must stay elevated once headers are ready but the body is still streaming"
    );

    let mut body = response.into_body();
    let first = tokio::time::timeout(Duration::from_secs(5), body.frame())
        .await
        .expect("first chunk should arrive")
        .expect("body should yield a frame")
        .expect("frame should not be an error");
    assert_eq!(&first.into_data().expect("data frame")[..], b"chunk-1");
    assert_eq!(
        telemetry::active_requests(&telemetry_handle),
        1,
        "active_requests must stay elevated between chunks"
    );

    let second = tokio::time::timeout(Duration::from_secs(5), body.frame())
        .await
        .expect("second chunk should arrive")
        .expect("body should yield a frame")
        .expect("frame should not be an error");
    assert_eq!(&second.into_data().expect("data frame")[..], b"chunk-2");

    let end = tokio::time::timeout(Duration::from_secs(5), body.frame())
        .await
        .expect("stream should end promptly after the second chunk");
    assert!(end.is_none(), "stream should end after the second chunk");

    assert_eq!(
        telemetry::active_requests(&telemetry_handle),
        0,
        "active_requests must drop back to 0 once the stream actually ends"
    );

    let line = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(line) = captured
                .lock()
                .expect("captured telemetry should not be poisoned")
                .first()
                .cloned()
            {
                break line;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("RequestEnd telemetry line should be emitted once the stream ends");
    let record: serde_json::Value =
        serde_json::from_str(&line).expect("telemetry output should be valid JSON");
    assert_eq!(record["status"], 200);

    peer_server.abort();
}

#[tokio::test]
async fn metering_outbox_retry_drain_removes_successfully_exported_records() {
    let state = build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    );

    state
        .core_store
        .append_outbox(
            store::CoreStoreBucket::MeteringOutbox,
            br#"{"trace_id":"retry","sampled":true}"#,
        )
        .expect("metering outbox record should persist");

    assert_eq!(
        state
            .core_store
            .peek_outbox(store::CoreStoreBucket::MeteringOutbox, 16)
            .expect("metering outbox should be readable")
            .len(),
        1
    );

    let drained = drain_metering_outbox_once(&state, 16)
        .await
        .expect("metering outbox retry drain should succeed");

    assert_eq!(drained, 1);
    assert!(
        state
            .core_store
            .peek_outbox(store::CoreStoreBucket::MeteringOutbox, 16)
            .expect("metering outbox should be readable after drain")
            .is_empty(),
        "successful retry export should delete durable outbox entries"
    );
}

#[tokio::test]
async fn metering_exporter_drains_sampled_records_off_request_path() {
    use std::time::Duration;

    let metering_dir = unique_test_dir("tachyon-metering-export");
    let (export_sender, export_receiver) = mpsc::channel(TELEMETRY_EXPORT_QUEUE_CAPACITY);
    let telemetry = telemetry::init_test_telemetry_with_emitter(move |line| {
        export_sender.try_send(line).is_ok()
    });
    let config = IntegrityConfig {
        telemetry_sample_rate: 1.0,
        routes: vec![
            IntegrityRoute::user_with_secrets(DEFAULT_ROUTE, &["DB_PASS"]),
            metering_test_route(&metering_dir),
        ],
        ..IntegrityConfig::default_sealed()
    };
    let state = build_test_state(config, telemetry);
    spawn_metering_exporter(state.clone(), export_receiver);
    let app = build_app(state);

    let response = app
        .oneshot(
            Request::post("/api/guest-example")
                .body(Body::from("Hello Lean FaaS!"))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::OK);

    let metering_file = metering_dir.join("metering.ndjson");
    let contents = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(contents) = fs::read_to_string(&metering_file) {
                if !contents.trim().is_empty() {
                    break contents;
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("metering exporter should flush a batch");

    assert!(contents.contains("\"path\":\"/api/guest-example\""));
    assert!(contents.contains("\"sampled\":true"));
    assert!(contents.contains("\"fuel_consumed\":"));

    let _ = fs::remove_dir_all(metering_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn tcp_layer4_listener_echoes_and_releases_route_permit() {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let port = free_tcp_port();
    let route = tcp_echo_test_route(1);
    let config = validate_integrity_config(IntegrityConfig {
        host_address: "127.0.0.1:8080".to_owned(),
        layer4: IntegrityLayer4Config {
            tcp: vec![IntegrityTcpBinding {
                port,
                target: "guest-tcp-echo".to_owned(),
            }],
            udp: Vec::new(),
        },
        routes: vec![route.clone()],
        ..IntegrityConfig::default_sealed()
    })
    .expect("TCP Layer 4 config should validate");
    let state = build_test_state(config, telemetry::init_test_telemetry());
    let listeners = start_tcp_layer4_listeners(state.clone())
        .await
        .expect("TCP Layer 4 listener should start");
    let listener_addr = listeners
        .first()
        .expect("one TCP Layer 4 listener should be started")
        .local_addr;

    let mut stream = tokio::net::TcpStream::connect(listener_addr)
        .await
        .expect("TCP client should connect");
    stream
        .write_all(b"ping over tcp")
        .await
        .expect("TCP client should write");
    stream
        .shutdown()
        .await
        .expect("TCP client should close write");

    let mut echoed = Vec::new();
    stream
        .read_to_end(&mut echoed)
        .await
        .expect("TCP client should read echoed bytes");
    assert_eq!(echoed, b"ping over tcp");

    let runtime = state.runtime.load_full();
    let control = runtime
        .concurrency_limits
        .get(&route.path)
        .expect("TCP route should have a limiter");
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if control.semaphore.available_permits() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("TCP Layer 4 permit should be released after disconnect");

    for listener in listeners {
        listener.join_handle.abort();
        let _ = listener.join_handle.await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn tcp_layer4_connection_handler_echoes_payload() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let route = tcp_echo_test_route(1);
    let config = validate_integrity_config(IntegrityConfig {
        routes: vec![route.clone()],
        ..IntegrityConfig::default_sealed()
    })
    .expect("TCP Layer 4 config should validate");
    let state = build_test_state(config, telemetry::init_test_telemetry());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener should bind");
    let listener_addr = listener
        .local_addr()
        .expect("test listener should expose a local address");

    let client = tokio::spawn(async move {
        let mut stream = tokio::net::TcpStream::connect(listener_addr)
            .await
            .expect("TCP client should connect");
        stream
            .write_all(b"ping over tcp")
            .await
            .expect("TCP client should write");
        stream
            .shutdown()
            .await
            .expect("TCP client should close write");

        let mut echoed = Vec::new();
        stream
            .read_to_end(&mut echoed)
            .await
            .expect("TCP client should read echoed bytes");
        echoed
    });

    let (server_stream, _) = listener.accept().await.expect("listener should accept");
    handle_tcp_layer4_connection(state, Arc::new(route), server_stream)
        .await
        .expect("TCP Layer 4 connection should complete");

    assert_eq!(
        client.await.expect("client task should finish"),
        b"ping over tcp"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tcp_layer4_listener_streams_echo_before_client_eof() {
    use std::time::Duration;

    let port = free_tcp_port();
    let route = tcp_echo_test_route(1);
    let config = validate_integrity_config(IntegrityConfig {
        host_address: "127.0.0.1:8080".to_owned(),
        layer4: IntegrityLayer4Config {
            tcp: vec![IntegrityTcpBinding {
                port,
                target: "guest-tcp-echo".to_owned(),
            }],
            udp: Vec::new(),
        },
        routes: vec![route],
        ..IntegrityConfig::default_sealed()
    })
    .expect("TCP Layer 4 config should validate");
    let state = build_test_state(config, telemetry::init_test_telemetry());
    let listeners = start_tcp_layer4_listeners(state)
        .await
        .expect("TCP Layer 4 listener should start");
    let listener_addr = listeners
        .first()
        .expect("one TCP Layer 4 listener should be started")
        .local_addr;

    let trailing = std::thread::spawn(move || {
        use std::io::{Read, Write};

        let mut stream =
            std::net::TcpStream::connect(listener_addr).expect("TCP client should connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("TCP client should set a read timeout");
        stream
            .write_all(b"ping")
            .expect("TCP client should write first chunk");

        let mut first_chunk = [0_u8; 4];
        stream
            .read_exact(&mut first_chunk)
            .expect("TCP listener should echo before client EOF");
        assert_eq!(&first_chunk, b"ping");

        stream
            .write_all(b" pong")
            .expect("TCP client should write second chunk");

        let mut second_chunk = [0_u8; 5];
        stream
            .read_exact(&mut second_chunk)
            .expect("TCP listener should keep streaming echoed chunks");
        assert_eq!(&second_chunk, b" pong");

        stream
            .shutdown(std::net::Shutdown::Write)
            .expect("TCP client should close write side");

        let mut trailing = Vec::new();
        stream
            .read_to_end(&mut trailing)
            .expect("TCP client should drain trailing bytes");
        trailing
    })
    .join()
    .expect("TCP client thread should finish");
    assert!(trailing.is_empty());

    for listener in listeners {
        listener.join_handle.abort();
        let _ = listener.join_handle.await;
    }
}
