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
    handle_tcp_layer4_connection(state, route, server_stream)
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
