use super::support_and_cache::*;
use crate::*;

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn uds_fast_path_registration_publishes_socket_metadata() {
    let discovery_dir = unique_test_dir("tachyon-uds-discovery");
    let registry = Arc::new(UdsFastPathRegistry::with_discovery_dir(
        discovery_dir.clone(),
    ));
    let config = IntegrityConfig {
        host_address: "127.0.0.1:19090".to_owned(),
        ..IntegrityConfig::default_sealed()
    };
    let app = axum::Router::new().route("/ping", axum::routing::get(|| async { "ok" }));
    let server = start_uds_fast_path_listener(app, &config, Arc::clone(&registry))
        .expect("UDS listener should register")
        .expect("UDS listener should start on Unix");

    tokio::time::sleep(Duration::from_millis(50)).await;

    let metadata_files = fs::read_dir(&discovery_dir)
        .expect("discovery dir should exist")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    assert_eq!(metadata_files.len(), 1);

    let metadata: UdsPeerMetadata =
        serde_json::from_slice(&fs::read(&metadata_files[0]).expect("metadata should be readable"))
            .expect("metadata should parse");
    assert_eq!(metadata.ip, "127.0.0.1");
    assert!(
        Path::new(&metadata.socket_path).exists(),
        "published UDS socket should exist"
    );

    server.abort();
    let _ = server.await;
    drop(registry);
    let _ = fs::remove_dir_all(discovery_dir);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn outbound_http_uds_fast_path_supports_post_body_and_reuses_client() {
    use axum::routing::post;

    async fn echo(body: Bytes) -> Bytes {
        body
    }

    let discovery_dir = unique_test_dir("tachyon-uds-outbound-post");
    let registry = Arc::new(UdsFastPathRegistry::with_discovery_dir(
        discovery_dir.clone(),
    ));
    let mut config = IntegrityConfig::default_sealed();
    config.host_address = "127.0.0.1:19292".to_owned();
    let app = axum::Router::new().route("/echo", post(echo));
    let server = start_uds_fast_path_listener(app, &config, Arc::clone(&registry))
        .expect("UDS listener should register")
        .expect("UDS listener should start on Unix");

    tokio::time::sleep(Duration::from_millis(50)).await;

    let first_registry = Arc::clone(&registry);
    let first_response = tokio::task::spawn_blocking(move || {
        send_blocking_uds_fast_path_request(
            first_registry.as_ref(),
            "http://127.0.0.1:19292/echo",
            &reqwest::Method::POST,
            &[("x-test".to_owned(), "uds".to_owned())],
            b"payload",
        )
    })
    .await
    .expect("blocking UDS request should not panic")
    .expect("UDS fast-path should handle POST");

    let second_registry = Arc::clone(&registry);
    let second_response = tokio::task::spawn_blocking(move || {
        send_blocking_uds_fast_path_request(
            second_registry.as_ref(),
            "http://127.0.0.1:19292/echo",
            &reqwest::Method::POST,
            &[],
            b"again",
        )
    })
    .await
    .expect("blocking UDS request should not panic")
    .expect("UDS fast-path should handle repeated POST");

    server.abort();
    let _ = server.await;

    assert_eq!(first_response.status, StatusCode::OK.as_u16());
    assert_eq!(first_response.body, b"payload");
    assert_eq!(second_response.status, StatusCode::OK.as_u16());
    assert_eq!(second_response.body, b"again");
    assert_eq!(
        registry
            .blocking_clients
            .lock()
            .expect("UDS blocking client cache should not be poisoned")
            .len(),
        1,
        "outbound HTTP should reuse a cached blocking UDS client"
    );
    let _ = fs::remove_dir_all(discovery_dir);
}

#[tokio::test]
async fn graceful_shutdown_waits_for_in_flight_requests() {
    use axum::routing::get;
    use tokio::sync::Notify;

    async fn slow_handler(State(started): State<Arc<Notify>>) -> &'static str {
        started.notify_one();
        tokio::time::sleep(Duration::from_millis(150)).await;
        "done"
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener
        .local_addr()
        .expect("listener should expose an address");
    let started = Arc::new(Notify::new());
    let app = Router::new()
        .route("/slow", get(slow_handler))
        .with_state(Arc::clone(&started));

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("server should shut down cleanly");
    });

    let request = tokio::spawn(async move {
        Client::new()
            .get(format!("http://{address}/slow"))
            .send()
            .await
            .expect("request should complete")
    });

    started.notified().await;
    let _ = shutdown_tx.send(());

    let response = request.await.expect("request task should complete");
    let status = response.status();
    let body = response
        .text()
        .await
        .expect("response body should be readable");

    server.await.expect("server task should complete");

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "done");
}

#[test]
fn error_response_normalizes_resource_limit_failures() {
    let config = IntegrityConfig::default_sealed();
    let response = ExecutionError::ResourceLimitExceeded {
        kind: ResourceLimitKind::Memory,
        detail: "guest exceeded its memory quota".to_string(),
    }
    .into_response(&config);

    assert_eq!(
        response,
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            config.resource_limit_response,
        )
    );
}
