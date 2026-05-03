use super::support_and_cache::*;
use crate::*;

#[tokio::test]
async fn router_returns_guest_stdout_for_post_request() {
    let app = build_app(build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
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

    let body = response
        .into_body()
        .collect()
        .await
        .expect("response body should collect")
        .to_bytes();

    assert_eq!(
        String::from_utf8_lossy(&body).trim(),
        expected_guest_example_body("FaaS received: Hello Lean FaaS!")
    );
}

#[tokio::test]
async fn router_accepts_get_requests() {
    let app = build_app(build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    ));
    let response = app
        .oneshot(
            Request::get("/api/guest-example")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::OK);

    let body = response
        .into_body()
        .collect()
        .await
        .expect("response body should collect")
        .to_bytes();

    assert_eq!(
        String::from_utf8_lossy(&body).trim(),
        expected_guest_example_body("FaaS received an empty payload")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn grpc_http2_route_returns_protobuf_body_and_grpc_status_trailer() {
    let config = validate_integrity_config(IntegrityConfig {
        routes: vec![targeted_route(
            "/grpc/hello",
            vec![weighted_target("guest-grpc", 100)],
        )],
        ..IntegrityConfig::default_sealed()
    })
    .expect("gRPC route config should validate");
    let app = build_app(build_test_state(config, telemetry::init_test_telemetry()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("gRPC test listener should bind");
    let address = listener
        .local_addr()
        .expect("gRPC test listener should expose an address");
    let server = tokio::spawn(async move {
        serve_http_listener(listener, app)
            .await
            .expect("gRPC test server should stay healthy");
    });

    let stream = tokio::net::TcpStream::connect(address)
        .await
        .expect("gRPC client should connect");
    let (mut sender, connection) =
        hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(stream))
            .await
            .expect("HTTP/2 handshake should succeed");
    let connection_task = tokio::spawn(async move {
        connection
            .await
            .expect("HTTP/2 connection should stay healthy");
    });

    let request_body = encode_test_grpc_message(&TestGrpcHelloRequest {
        name: "Tachyon".to_owned(),
    });
    let response = sender
        .send_request(
            Request::builder()
                .method("POST")
                .uri(format!("http://{address}/grpc/hello"))
                .version(hyper::Version::HTTP_2)
                .header("content-type", "application/grpc")
                .header("te", "trailers")
                .body(Full::new(Bytes::from(request_body)))
                .expect("gRPC request should build"),
        )
        .await
        .expect("gRPC request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.version(), hyper::Version::HTTP_2);
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("application/grpc")
    );

    let mut body = response.into_body();
    let mut framed_payload = Vec::new();
    let mut trailers = None;
    while let Some(frame) = body.frame().await {
        let frame = frame.expect("HTTP/2 response frame should be readable");
        if let Some(data) = frame.data_ref() {
            framed_payload.extend_from_slice(data);
        }
        if let Some(frame_trailers) = frame.trailers_ref() {
            trailers = Some(frame_trailers.clone());
        }
    }

    let decoded = decode_test_grpc_message::<TestGrpcHelloResponse>(&framed_payload);
    assert_eq!(decoded.message, "Hello, Tachyon!");
    assert_eq!(
        trailers
            .as_ref()
            .and_then(|trailers| trailers.get("grpc-status"))
            .and_then(|value| value.to_str().ok()),
        Some("0")
    );

    server.abort();
    connection_task.abort();
    let _ = server.await;
    let _ = connection_task.await;
}

#[tokio::test(flavor = "multi_thread")]
async fn async_logger_exports_log_storm_without_leaking_logs_into_response() {
    let log_dir = unique_test_dir("tachyon-async-logger");
    let config = validate_integrity_config(IntegrityConfig {
        routes: vec![log_storm_test_route(), logger_test_route(&log_dir)],
        ..IntegrityConfig::default_sealed()
    })
    .expect("async logger config should validate");
    let app = build_app(build_test_state(config, telemetry::init_test_telemetry()));

    let response = app
        .oneshot(
            Request::post("/api/guest-log-storm")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::OK);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("response body should collect")
        .to_bytes();
    assert_eq!(String::from_utf8_lossy(&body).trim(), "storm-complete");

    let log_file = log_dir.join("guest-logs.ndjson");
    for _ in 0..30 {
        if log_file.exists()
            && fs::metadata(&log_file)
                .map(|metadata| metadata.len() > 0)
                .unwrap_or(false)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let contents = fs::read_to_string(&log_file).expect("logger output should exist");
    assert!(contents.contains("\"target_name\":\"guest-log-storm\""));
    assert!(contents.contains("\"message\":\"storm-"));

    let _ = fs::remove_dir_all(log_dir);
}

#[test]
#[ignore = "manual performance comparison for async logging validation"]
fn async_log_capture_is_faster_than_sync_file_capture() {
    let route = log_storm_test_route();
    let config = validate_integrity_config(IntegrityConfig {
        max_stdout_bytes: 16 * 1024 * 1024,
        routes: vec![route.clone()],
        ..IntegrityConfig::default_sealed()
    })
    .expect("benchmark config should validate");
    let engine = build_test_engine(&config);
    let async_execution = GuestExecutionContext {
        config: config.clone(),
        sampled_execution: false,
        runtime_telemetry: telemetry::init_test_telemetry(),
        async_log_sender: test_log_sender(),
        secret_access: SecretAccess::default(),
        request_headers: HeaderMap::new(),
        host_identity: test_host_identity(44),
        storage_broker: Arc::new(StorageBrokerManager::default()),
        bridge_manager: Arc::new(BridgeManager::default()),
        telemetry: None,
        concurrency_limits: build_concurrency_limits(&config),
        propagated_headers: Vec::new(),
        route_overrides: test_route_overrides(),
        host_load: test_host_load(),
        #[cfg(feature = "ai-inference")]
        ai_runtime: test_ai_runtime(&config),
        instance_pool: None,
    };

    let request = GuestRequest::new("POST", "/api/guest-log-storm", Bytes::new());

    let async_start = Instant::now();
    let async_result = execute_guest(
        &engine,
        "guest-log-storm",
        request.clone(),
        &route,
        async_execution,
    )
    .expect("async log capture should succeed");
    let async_elapsed = async_start.elapsed();

    let sync_execution = GuestExecutionContext {
        config: config.clone(),
        sampled_execution: false,
        runtime_telemetry: telemetry::init_test_telemetry(),
        async_log_sender: test_log_sender(),
        secret_access: SecretAccess::default(),
        request_headers: HeaderMap::new(),
        host_identity: test_host_identity(45),
        storage_broker: Arc::new(StorageBrokerManager::default()),
        bridge_manager: Arc::new(BridgeManager::default()),
        telemetry: None,
        concurrency_limits: build_concurrency_limits(&config),
        propagated_headers: Vec::new(),
        route_overrides: test_route_overrides(),
        host_load: test_host_load(),
        #[cfg(feature = "ai-inference")]
        ai_runtime: test_ai_runtime(&config),
        instance_pool: None,
    };
    let sync_start = Instant::now();
    let sync_result = execute_legacy_guest_with_sync_file_capture(
        &engine,
        "guest-log-storm",
        request.body,
        &route,
        &sync_execution,
    )
    .expect("sync log capture should succeed");
    let sync_elapsed = sync_start.elapsed();

    let GuestExecutionOutcome {
        output: GuestExecutionOutput::LegacyStdout(async_stdout),
        ..
    } = async_result
    else {
        unreachable!("async benchmark should return legacy stdout");
    };
    let GuestExecutionOutcome {
        output: GuestExecutionOutput::LegacyStdout(sync_stdout),
        ..
    } = sync_result
    else {
        unreachable!("sync benchmark should return legacy stdout");
    };

    assert_eq!(
        String::from_utf8_lossy(&async_stdout).trim(),
        "storm-complete"
    );
    assert_eq!(
        String::from_utf8_lossy(&sync_stdout).trim(),
        "storm-complete"
    );
    eprintln!(
            "guest-log-storm benchmark: async_capture={async_elapsed:?}, sync_file_capture={sync_elapsed:?}"
        );
    assert!(
            async_elapsed < sync_elapsed,
            "expected async capture to beat sync file capture (async={async_elapsed:?}, sync={sync_elapsed:?})"
        );
}

#[tokio::test]
async fn router_rejects_exhausted_hop_limit_header() {
    let app = build_app(build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    ));
    let response = app
        .oneshot(
            Request::get("/api/guest-example")
                .header(HOP_LIMIT_HEADER, "0")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::LOOP_DETECTED);

    let body = response
        .into_body()
        .collect()
        .await
        .expect("response body should collect")
        .to_bytes();

    assert!(
        String::from_utf8_lossy(&body).contains("Routing loop detected"),
        "unexpected loop-detected response body: {:?}",
        body
    );
}

#[tokio::test]
async fn router_rejects_unsealed_routes() {
    let app = build_app(build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    ));
    let response = app
        .oneshot(
            Request::post("/api/guest-malicious")
                .body(Body::from("blocked"))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn router_returns_service_unavailable_when_route_concurrency_is_exhausted() {
    let config = IntegrityConfig::default_sealed();
    let runtime = RuntimeState {
        engine: build_test_engine(&config),
        metered_engine: build_test_metered_engine(&config),
        route_registry: Arc::new(
            RouteRegistry::build(&config).expect("route registry should build"),
        ),
        batch_target_registry: Arc::new(
            BatchTargetRegistry::build(&config).expect("batch target registry should build"),
        ),
        concurrency_limits: Arc::new(HashMap::from([(
            DEFAULT_ROUTE.to_owned(),
            Arc::new(RouteExecutionControl::from_limits(0, 0)),
        )])),
        instance_pool: Arc::new(
            moka::sync::Cache::builder()
                .max_capacity(INSTANCE_POOL_DEFAULT_CAPACITY)
                .time_to_idle(INSTANCE_POOL_IDLE_TIMEOUT)
                .build(),
        ),
        #[cfg(feature = "ai-inference")]
        ai_runtime: test_ai_runtime(&config),
        config,
    };
    let core_store_manifest = unique_test_dir("app-state-manifest").join("integrity.lock");
    let buffered_requests = Arc::new(
        BufferedRequestManager::new(buffered_request_spool_dir(&core_store_manifest))
            .expect("test buffered request manager should initialize"),
    );
    let core_store = Arc::new(
        store::CoreStore::open(&core_store_path(&core_store_manifest))
            .expect("test core store should open"),
    );
    let state = AppState {
        runtime: Arc::new(ArcSwap::from_pointee(runtime)),
        draining_runtimes: Arc::new(Mutex::new(Vec::new())),
        http_client: Client::new(),
        async_log_sender: test_log_sender(),
        secrets_vault: SecretsVault::load(),
        host_identity: test_host_identity(22),
        uds_fast_path: Arc::new(new_uds_fast_path_registry()),
        storage_broker: Arc::new(StorageBrokerManager::new(Arc::clone(&core_store))),
        bridge_manager: Arc::new(BridgeManager::default()),
        core_store,
        buffered_requests,
        volume_manager: Arc::new(VolumeManager::default()),
        route_overrides: Arc::new(ArcSwap::from_pointee(HashMap::new())),
        peer_capabilities: Arc::new(Mutex::new(HashMap::new())),
        host_capabilities: Capabilities::detect(),
        host_load: Arc::new(HostLoadCounters::default()),
        memory_governor: Arc::new(memory_governor::MemoryGovernor::new(1_000, 75, 90)),
        telemetry: telemetry::init_test_telemetry(),
        tls_manager: Arc::new(tls_runtime::TlsManager::default()),
        mtls_gateway: None,
        auth_manager: Arc::new(
            auth::AuthManager::new(&core_store_manifest)
                .expect("test auth manager should initialize"),
        ),
        enrollment_manager: Arc::new(node_enrollment::EnrollmentManager::new()),
        config_updates: broadcast::channel(CONFIG_UPDATE_CHANNEL_CAPACITY).0,
        manifest_path: core_store_manifest,
        background_workers: Arc::new(BackgroundWorkerManager::default()),
    };
    spawn_buffered_request_replayer(state.clone());
    spawn_pressure_monitor(state.clone());
    let app = build_app(state);

    let response = app
        .oneshot(
            Request::post("/api/guest-example")
                .body(Body::from("blocked"))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let body = response
        .into_body()
        .collect()
        .await
        .expect("response body should collect")
        .to_bytes();

    assert!(String::from_utf8_lossy(&body).contains("buffered request timed out"));
}

#[test]
fn buffered_request_manager_spills_to_disk_after_ram_capacity() {
    let spool_dir = unique_test_dir("buffered-request-spool");
    let manager = BufferedRequestManager::new(spool_dir)
        .expect("test buffered request manager should initialize");
    let request = BufferedRouteRequest {
        route_path: DEFAULT_ROUTE.to_owned(),
        selected_module: "guest-example".to_owned(),
        method: "POST".to_owned(),
        uri: "http://localhost/api/guest-example".to_owned(),
        headers: Vec::new(),
        body: b"payload".to_vec(),
        trailers: Vec::new(),
        hop_limit: DEFAULT_HOP_LIMIT,
        trace_id: None,
        sampled_execution: false,
    };

    for _ in 0..=BUFFER_RAM_REQUEST_CAPACITY {
        let _ = manager
            .enqueue(request.clone())
            .expect("buffered request should enqueue");
    }

    assert_eq!(manager.pending_count(), BUFFER_RAM_REQUEST_CAPACITY + 1);
    assert_eq!(manager.disk_spill_count(), 1);
}

#[test]
fn system_guest_requires_system_route_role() {
    let config = IntegrityConfig::default_sealed();
    let engine = build_test_engine(&config);
    let route = IntegrityRoute::user("/metrics");
    #[cfg(feature = "ai-inference")]
    let ai_runtime = test_ai_runtime(&config);
    let error = execute_guest(
        &engine,
        "metrics",
        GuestRequest::new("GET", "/metrics", Bytes::new()),
        &route,
        GuestExecutionContext {
            config,
            sampled_execution: false,
            runtime_telemetry: telemetry::init_test_telemetry(),
            async_log_sender: test_log_sender(),
            secret_access: SecretAccess::default(),
            request_headers: HeaderMap::new(),
            host_identity: test_host_identity(34),
            storage_broker: Arc::new(StorageBrokerManager::default()),
            bridge_manager: Arc::new(BridgeManager::default()),
            telemetry: None,
            concurrency_limits: build_concurrency_limits(&IntegrityConfig::default_sealed()),
            propagated_headers: Vec::new(),
            route_overrides: test_route_overrides(),
            host_load: test_host_load(),
            #[cfg(feature = "ai-inference")]
            ai_runtime,
            instance_pool: None,
        },
    )
    .expect_err("privileged metrics guest should fail as a user route");

    match error {
        ExecutionError::Internal(message) => {
            assert!(message.contains("telemetry-reader") || message.contains("telemetry_reader"));
        }
        other => unreachable!("unexpected error variant: {other:?}"),
    }
}

#[tokio::test]
async fn router_returns_system_metrics_for_privileged_route() {
    let app = build_app(build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    ));

    let response = app
        .oneshot(
            Request::get("/metrics")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::OK);

    let body = response
        .into_body()
        .collect()
        .await
        .expect("response body should collect")
        .to_bytes();
    let text = String::from_utf8_lossy(&body);

    assert!(text.contains("tachyon_requests_total"));
    assert!(text.contains("tachyon_active_requests"));
}

#[tokio::test]
async fn router_returns_scaling_metrics_for_privileged_route() {
    let config = autoscaling_test_config(false);
    let state = build_test_state(config, telemetry::init_test_telemetry());
    let runtime = state.runtime.load_full();
    runtime
        .concurrency_limits
        .get("/api/guest-call-legacy")
        .expect("legacy route should have a limiter")
        .pending_waiters
        .store(7, Ordering::SeqCst);
    let app = build_app(state);

    let response = app
        .oneshot(
            Request::get("/metrics/scaling")
                .body(Body::empty())
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::OK);

    let body = response
        .into_body()
        .collect()
        .await
        .expect("response body should collect")
        .to_bytes();
    let text = String::from_utf8_lossy(&body);

    assert!(text.contains("tachyon_pending_requests"));
    assert!(text.contains("route=\"/api/guest-call-legacy\""));
    assert!(text.contains(" 7"));
}

#[tokio::test]
async fn router_buffers_ai_generation_and_exposes_job_status() {
    let app = build_app(build_test_state(
        IntegrityConfig::default_sealed(),
        telemetry::init_test_telemetry(),
    ));

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/generate")
                .body(Body::from("hello model"))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body should collect")
        .to_bytes();
    let payload: Value = serde_json::from_slice(&body).expect("accepted body should be JSON");
    let job_id = payload["job_id"].as_str().expect("job id should exist");

    let mut status = None;
    for _ in 0..10 {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/jobs/{job_id}"))
                    .body(Body::empty())
                    .expect("status request should build"),
            )
            .await
            .expect("status request should complete");
        assert_eq!(response.status(), StatusCode::OK);
        let body = response
            .into_body()
            .collect()
            .await
            .expect("status body should collect")
            .to_bytes();
        let value: Value = serde_json::from_slice(&body).expect("status should be JSON");
        if value["status"] == "completed" {
            status = Some(value);
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let status = status.expect("job should complete");
    assert_eq!(status["output"], "generated:hello model");
}
