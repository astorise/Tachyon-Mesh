use super::support_and_cache::*;
use crate::*;

#[tokio::test]
async fn reload_runtime_from_disk_swaps_in_new_routes() {
    let temp_dir = unique_test_dir("graceful-reload-state");
    let manifest_path = temp_dir.join("integrity.lock");
    let initial = IntegrityConfig {
        routes: vec![IntegrityRoute::user(DEFAULT_ROUTE)],
        ..IntegrityConfig::default_sealed()
    };
    write_test_manifest(&manifest_path, &initial, 11);
    let state = build_test_state_with_manifest(
        initial,
        telemetry::init_test_telemetry(),
        manifest_path.clone(),
    );

    let mut reloaded = IntegrityConfig {
        routes: vec![IntegrityRoute::user(DEFAULT_ROUTE)],
        ..IntegrityConfig::default_sealed()
    };
    reloaded
        .routes
        .push(IntegrityRoute::user("/api/guest-loop"));
    write_test_manifest(&manifest_path, &reloaded, 12);

    reload_runtime_from_disk(&state)
        .await
        .expect("runtime should reload from manifest");

    let runtime = state.runtime.load_full();
    assert!(runtime.config.sealed_route("/api/guest-loop").is_some());
    assert!(runtime.concurrency_limits.contains_key("/api/guest-loop"));
}

#[tokio::test]
async fn reload_runtime_from_disk_keeps_previous_state_on_invalid_manifest() {
    let temp_dir = unique_test_dir("graceful-reload-invalid");
    let manifest_path = temp_dir.join("integrity.lock");
    let initial = IntegrityConfig {
        routes: vec![IntegrityRoute::user(DEFAULT_ROUTE)],
        ..IntegrityConfig::default_sealed()
    };
    write_test_manifest(&manifest_path, &initial, 13);
    let state = build_test_state_with_manifest(
        initial,
        telemetry::init_test_telemetry(),
        manifest_path.clone(),
    );

    fs::write(&manifest_path, "{ invalid json").expect("invalid manifest should be written");

    let error = reload_runtime_from_disk(&state)
        .await
        .expect_err("invalid manifest should not replace the runtime");

    assert!(error
        .to_string()
        .contains("failed to parse integrity manifest"));
    let runtime = state.runtime.load_full();
    assert!(runtime.config.sealed_route(DEFAULT_ROUTE).is_some());
    assert!(runtime.config.sealed_route("/api/guest-loop").is_none());
}

#[tokio::test]
async fn reload_runtime_from_disk_drains_previous_generation_until_response_flush() {
    let temp_dir = unique_test_dir("graceful-drain");
    let manifest_path = temp_dir.join("integrity.lock");
    let initial = IntegrityConfig {
        routes: vec![draining_test_route("guest-flaky", "1.0.0")],
        ..IntegrityConfig::default_sealed()
    };
    write_test_manifest(&manifest_path, &initial, 31);
    let state = build_test_state_with_manifest(
        initial,
        telemetry::init_test_telemetry(),
        manifest_path.clone(),
    );
    let app = build_app(state.clone());

    let slow_request = {
        let app = app.clone();
        tokio::spawn(async move {
            app.oneshot(
                Request::post("/api/drain")
                    .body(Body::from("sleep:250"))
                    .expect("slow request should build"),
            )
            .await
            .expect("slow request should complete")
        })
    };

    tokio::time::sleep(Duration::from_millis(50)).await;
    let initial_runtime = state.runtime.load_full();
    let initial_control = initial_runtime
        .concurrency_limits
        .get("/api/drain")
        .cloned()
        .expect("initial route control should exist");
    assert_eq!(initial_control.active_request_count(), 1);
    drop(initial_runtime);

    let reloaded = IntegrityConfig {
        routes: vec![draining_test_route("guest-example", "2.0.0")],
        ..IntegrityConfig::default_sealed()
    };
    write_test_manifest(&manifest_path, &reloaded, 32);
    reload_runtime_from_disk(&state)
        .await
        .expect("runtime should reload from manifest");

    let fresh_response = app
        .clone()
        .oneshot(
            Request::post("/api/drain")
                .body(Body::from("hello-v2"))
                .expect("fresh request should build"),
        )
        .await
        .expect("fresh request should complete");
    let fresh_body = fresh_response
        .into_body()
        .collect()
        .await
        .expect("fresh response body should collect")
        .to_bytes();
    assert!(
        String::from_utf8_lossy(&fresh_body).contains("FaaS received: hello-v2"),
        "unexpected fresh body: {:?}",
        fresh_body
    );

    let slow_response = slow_request
        .await
        .expect("slow request task should join cleanly");
    assert_eq!(
        state
            .draining_runtimes
            .lock()
            .expect("draining runtime list should not be poisoned")
            .len(),
        1
    );
    run_draining_runtime_reaper_tick(&state);
    assert_eq!(
        state
            .draining_runtimes
            .lock()
            .expect("draining runtime list should not be poisoned")
            .len(),
        1,
        "the old generation should remain while its response body is still owned"
    );
    assert_eq!(initial_control.active_request_count(), 1);

    let slow_body = slow_response
        .into_body()
        .collect()
        .await
        .expect("slow response body should collect")
        .to_bytes();
    assert_eq!(slow_body, Bytes::from_static(b"slept:250"));
    run_draining_runtime_reaper_tick(&state);
    assert_eq!(initial_control.active_request_count(), 0);
    assert_eq!(
        state
            .draining_runtimes
            .lock()
            .expect("draining runtime list should not be poisoned")
            .len(),
        0,
        "the old generation should be reaped once the response finishes flushing"
    );
}

#[test]
fn draining_runtime_reaper_forces_timeout_after_deadline() {
    let state = build_test_state(
        IntegrityConfig {
            routes: vec![draining_test_route("guest-flaky", "1.0.0")],
            ..IntegrityConfig::default_sealed()
        },
        telemetry::init_test_telemetry(),
    );
    let runtime = state.runtime.load_full();
    let control = runtime
        .concurrency_limits
        .get("/api/drain")
        .cloned()
        .expect("route control should exist");
    let _guard = control.begin_request();
    let draining_since = Instant::now()
        .checked_sub(DRAINING_ROUTE_TIMEOUT + Duration::from_secs(1))
        .expect("deadline subtraction should remain valid");
    runtime.mark_draining(draining_since);
    state
        .draining_runtimes
        .lock()
        .expect("draining runtime list should not be poisoned")
        .push(DrainingRuntime {
            runtime,
            draining_since,
        });

    run_draining_runtime_reaper_tick(&state);

    assert_eq!(
        state
            .draining_runtimes
            .lock()
            .expect("draining runtime list should not be poisoned")
            .len(),
        0
    );
    assert!(control.semaphore.is_closed());
}

#[tokio::test]
async fn run_mode_executes_gc_batch_target_and_deletes_stale_files() {
    let temp_dir = unique_test_dir("batch-gc");
    let cache_dir = temp_dir.join("cache");
    fs::create_dir_all(cache_dir.join("nested")).expect("cache directory should exist");
    let stale_file = cache_dir.join("nested").join("stale.txt");
    fs::write(&stale_file, "stale").expect("stale file should be written");

    let manifest_path = temp_dir.join("integrity.lock");
    let mut config = IntegrityConfig::default_sealed();
    config.routes.clear();
    config.batch_targets = vec![gc_batch_target(&cache_dir, 0)];
    write_test_manifest(&manifest_path, &config, 14);

    let success = execute_batch_target_from_manifest(manifest_path, "gc-job")
        .await
        .expect("batch target should execute successfully");

    assert!(success, "batch target should exit successfully");
    assert!(
        !stale_file.exists(),
        "batch GC target should delete stale files"
    );
}

#[test]
fn execute_guest_returns_component_response_payload() {
    let config = IntegrityConfig::default_sealed();
    let engine = build_test_engine(&config);
    let route = config
        .sealed_route("/api/guest-example")
        .expect("sealed route should exist")
        .clone();
    #[cfg(feature = "ai-inference")]
    let ai_runtime = test_ai_runtime(&config);
    let response = execute_guest(
        &engine,
        "guest-example",
        GuestRequest::new("POST", "/api/guest-example", "Hello Lean FaaS!"),
        &route,
        GuestExecutionContext {
            secret_access: SecretAccess::from_route(&route, &SecretsVault::load()),
            config,
            sampled_execution: false,
            runtime_telemetry: telemetry::init_test_telemetry(),
            async_log_sender: test_log_sender(),
            request_headers: HeaderMap::new(),
            host_identity: test_host_identity(30),
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
    .expect("guest execution should succeed");

    assert_eq!(
        response,
        GuestExecutionOutcome {
            output: GuestExecutionOutput::Http(GuestHttpResponse::new(
                StatusCode::OK,
                Bytes::from(expected_guest_example_body(
                    "FaaS received: Hello Lean FaaS!"
                )),
            )),
            fuel_consumed: None,
        }
    );
}

#[test]
fn execute_guest_falls_back_to_legacy_stdout_for_non_component_module() {
    let config = IntegrityConfig::default_sealed();
    let engine = build_test_engine(&config);
    let route = IntegrityRoute::user("/api/guest-call-legacy");
    #[cfg(feature = "ai-inference")]
    let ai_runtime = test_ai_runtime(&config);
    let response = execute_guest(
        &engine,
        "guest-call-legacy",
        GuestRequest::new("GET", "/api/guest-call-legacy", Bytes::new()),
        &route,
        GuestExecutionContext {
            config,
            sampled_execution: false,
            runtime_telemetry: telemetry::init_test_telemetry(),
            async_log_sender: test_log_sender(),
            secret_access: SecretAccess::default(),
            request_headers: HeaderMap::new(),
            host_identity: test_host_identity(31),
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
    .expect("legacy guest execution should succeed");

    assert_eq!(
        response,
        GuestExecutionOutcome {
            output: GuestExecutionOutput::LegacyStdout(Bytes::from(
                "MESH_FETCH:http://mesh/legacy-service/ping\n"
            )),
            fuel_consumed: None,
        }
    );
}

#[test]
fn execute_legacy_guest_reads_stdin_for_tcp_echo_module() {
    let config = IntegrityConfig::default_sealed();
    let engine = build_test_engine(&config);
    let route = tcp_echo_test_route(1);
    #[cfg(feature = "ai-inference")]
    let ai_runtime = test_ai_runtime(&config);
    let response = execute_guest(
        &engine,
        "guest-tcp-echo",
        GuestRequest::new(
            "TCP",
            "tcp://guest-tcp-echo",
            Bytes::from_static(b"ping over tcp"),
        ),
        &route,
        GuestExecutionContext {
            config,
            sampled_execution: false,
            runtime_telemetry: telemetry::init_test_telemetry(),
            async_log_sender: test_log_sender(),
            secret_access: SecretAccess::default(),
            request_headers: HeaderMap::new(),
            host_identity: test_host_identity(32),
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
    .expect("legacy guest execution should succeed");

    assert_eq!(
        response,
        GuestExecutionOutcome {
            output: GuestExecutionOutput::LegacyStdout(Bytes::from_static(b"ping over tcp")),
            fuel_consumed: None,
        }
    );
}

#[cfg(feature = "ai-inference")]
#[test]
fn execute_guest_ai_uses_preloaded_model_alias_and_returns_mock_text() {
    let mut route = IntegrityRoute::user("/api/guest-ai");
    route.models = vec![IntegrityModelBinding {
        alias: "llama3".to_owned(),
        path: "/models/llama3.gguf".to_owned(),
        device: ModelDevice::Cuda,
        qos: RouteQos::Standard,
    }];
    let config = IntegrityConfig {
        routes: vec![route.clone()],
        ..IntegrityConfig::default_sealed()
    };
    let engine = build_test_engine(&config);
    let ai_runtime = test_ai_runtime(&config);

    let response = execute_guest(
            &engine,
            "guest-ai",
            GuestRequest::new(
                "POST",
                "/api/guest-ai",
                Bytes::from_static(
                    br#"{"model":"llama3","shape":[1,4],"values":[1.0,2.0,3.0,4.0],"output_len":17,"response_kind":"text"}"#,
                ),
            ),
            &route,
            GuestExecutionContext {
                config,
                sampled_execution: false,
                runtime_telemetry: telemetry::init_test_telemetry(),
                async_log_sender: test_log_sender(),
                secret_access: SecretAccess::default(),
                request_headers: HeaderMap::new(),
                host_identity: test_host_identity(35),
                storage_broker: Arc::new(StorageBrokerManager::default()),
                bridge_manager: Arc::new(BridgeManager::default()),
                telemetry: None,
                concurrency_limits: build_concurrency_limits(&IntegrityConfig::default_sealed()),
                propagated_headers: Vec::new(),
                route_overrides: test_route_overrides(),
                host_load: test_host_load(),
                ai_runtime,
                instance_pool: None,
            },
        )
        .expect("AI guest execution should succeed");

    let GuestExecutionOutcome {
        output: GuestExecutionOutput::LegacyStdout(stdout),
        ..
    } = response
    else {
        unreachable!("AI guest should return legacy stdout");
    };

    let payload: Value = serde_json::from_slice(&stdout).expect("guest response should be JSON");
    assert_eq!(payload["model"], Value::String("llama3".to_owned()));
    assert_eq!(
        payload["text"],
        Value::String("MOCK_LLM_RESPONSE".to_owned())
    );
    assert_eq!(payload["output_bytes"], Value::from(17));
}

#[test]
fn execute_guest_persists_volume_data_for_component_guest() {
    let volume_dir = unique_test_dir("tachyon-volume-test");
    let route = volume_test_route(&volume_dir, false);
    let config = IntegrityConfig {
        routes: vec![route.clone()],
        ..IntegrityConfig::default_sealed()
    };
    let engine = build_test_engine(&config);
    #[cfg(feature = "ai-inference")]
    let ai_runtime = test_ai_runtime(&config);

    let save_response = execute_guest(
        &engine,
        "guest-volume",
        GuestRequest::new("POST", "/api/guest-volume", "Hello Stateful World"),
        &route,
        GuestExecutionContext {
            config: config.clone(),
            sampled_execution: false,
            runtime_telemetry: telemetry::init_test_telemetry(),
            async_log_sender: test_log_sender(),
            secret_access: SecretAccess::default(),
            request_headers: HeaderMap::new(),
            host_identity: test_host_identity(32),
            storage_broker: Arc::new(StorageBrokerManager::default()),
            bridge_manager: Arc::new(BridgeManager::default()),
            telemetry: None,
            concurrency_limits: build_concurrency_limits(&config),
            propagated_headers: Vec::new(),
            route_overrides: test_route_overrides(),
            host_load: test_host_load(),
            #[cfg(feature = "ai-inference")]
            ai_runtime: Arc::clone(&ai_runtime),
            instance_pool: None,
        },
    )
    .expect("volume guest should write successfully");

    assert_eq!(
        save_response,
        GuestExecutionOutcome {
            output: GuestExecutionOutput::Http(GuestHttpResponse::new(StatusCode::OK, "Saved",)),
            fuel_consumed: None,
        }
    );

    let read_response = execute_guest(
        &engine,
        "guest-volume",
        GuestRequest::new("GET", "/api/guest-volume", Bytes::new()),
        &route,
        GuestExecutionContext {
            config: config.clone(),
            sampled_execution: false,
            runtime_telemetry: telemetry::init_test_telemetry(),
            async_log_sender: test_log_sender(),
            secret_access: SecretAccess::default(),
            request_headers: HeaderMap::new(),
            host_identity: test_host_identity(33),
            storage_broker: Arc::new(StorageBrokerManager::default()),
            bridge_manager: Arc::new(BridgeManager::default()),
            telemetry: None,
            concurrency_limits: build_concurrency_limits(&config),
            propagated_headers: Vec::new(),
            route_overrides: test_route_overrides(),
            host_load: test_host_load(),
            #[cfg(feature = "ai-inference")]
            ai_runtime,
            instance_pool: None,
        },
    )
    .expect("volume guest should read successfully");

    assert_eq!(
        read_response,
        GuestExecutionOutcome {
            output: GuestExecutionOutput::Http(GuestHttpResponse::new(
                StatusCode::OK,
                "Hello Stateful World",
            )),
            fuel_consumed: None,
        }
    );
    assert_eq!(
        fs::read_to_string(volume_dir.join("state.txt")).expect("host volume file should exist"),
        "Hello Stateful World"
    );

    let _ = fs::remove_dir_all(volume_dir);
}
