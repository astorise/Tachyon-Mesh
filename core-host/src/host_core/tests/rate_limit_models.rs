use super::support_and_cache::*;
use crate::*;

#[test]
fn distributed_rate_limit_key_uses_first_forwarded_for_ip() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-forwarded-for",
        "203.0.113.10, 198.51.100.20"
            .parse()
            .expect("header should parse"),
    );

    let policy = DistributedRateLimitConfig {
        threshold: 100,
        window_seconds: 60,
        scope: DistributedRateLimitScope::Ip,
    };

    assert_eq!(
        distributed_rate_limit_key(&policy, &headers, &test_host_identity(31), "/api/protected")
            .expect("ip key should build"),
        "ip:203.0.113.10:/api/protected"
    );
}

#[test]
fn distributed_rate_limit_key_buckets_same_ip_by_tenant_identity() {
    let host_identity = test_host_identity(32);
    let policy = DistributedRateLimitConfig {
        threshold: 1,
        window_seconds: 60,
        scope: DistributedRateLimitScope::Tenant,
    };
    let mut headers_a = HeaderMap::new();
    let mut headers_b = HeaderMap::new();
    headers_a.insert(
        "x-forwarded-for",
        "203.0.113.10".parse().expect("header should parse"),
    );
    headers_b.insert(
        "x-forwarded-for",
        "203.0.113.10".parse().expect("header should parse"),
    );
    let now = unix_timestamp_seconds().expect("system clock should be available");
    let token_a = host_identity
        .sign_claims(&CallerIdentityClaims {
            route_path: "/api/protected".to_owned(),
            role: RouteRole::User,
            tenant_id: Some("tenant-a".to_owned()),
            token_id: Some("token-a".to_owned()),
            issued_at: now,
            expires_at: now.saturating_add(60),
        })
        .expect("tenant-a token should sign");
    let token_b = host_identity
        .sign_claims(&CallerIdentityClaims {
            route_path: "/api/protected".to_owned(),
            role: RouteRole::User,
            tenant_id: Some("tenant-b".to_owned()),
            token_id: Some("token-b".to_owned()),
            issued_at: now,
            expires_at: now.saturating_add(60),
        })
        .expect("tenant-b token should sign");
    headers_a.insert(
        TACHYON_IDENTITY_HEADER,
        format!("Bearer {token_a}")
            .parse()
            .expect("identity header should parse"),
    );
    headers_b.insert(
        TACHYON_IDENTITY_HEADER,
        format!("Bearer {token_b}")
            .parse()
            .expect("identity header should parse"),
    );

    let key_a = distributed_rate_limit_key(&policy, &headers_a, &host_identity, "/api/protected")
        .expect("tenant-a key should build");
    let key_b = distributed_rate_limit_key(&policy, &headers_b, &host_identity, "/api/protected")
        .expect("tenant-b key should build");

    assert_eq!(key_a, "tenant:tenant-a:/api/protected");
    assert_eq!(key_b, "tenant:tenant-b:/api/protected");
    assert_ne!(key_a, key_b);
}

#[test]
fn distributed_rate_limit_decision_rejects_denied_response() {
    let mut route = IntegrityRoute::user("/api/protected");
    route.distributed_rate_limit = Some(DistributedRateLimitConfig {
        threshold: 100,
        window_seconds: 60,
        scope: DistributedRateLimitScope::Ip,
    });
    let response = GuestHttpResponse::new(
        StatusCode::OK,
        Bytes::from_static(br#"{"allowed":false,"total":101}"#),
    );

    let rejection =
        distributed_rate_limit_decision(&route, response).expect("request should be rejected");

    assert_eq!(rejection.0, StatusCode::TOO_MANY_REQUESTS);
    assert!(rejection.1.contains("/api/protected"));
}

#[test]
fn distributed_rate_limit_bypasses_invalid_response_with_metric() {
    let route = IntegrityRoute::user("/api/protected");
    let before = distributed_rate_limit_bypass_total();
    let response = GuestHttpResponse::new(StatusCode::OK, Bytes::from_static(b"not-json"));

    assert!(distributed_rate_limit_decision(&route, response).is_none());
    assert_eq!(distributed_rate_limit_bypass_total(), before + 1);
}

#[test]
fn keda_pending_signal_prefers_internal_capacity_before_scale_out() {
    let control = RouteExecutionControl::from_limits(0, 4);
    control.pending_waiters.store(3, Ordering::SeqCst);
    control.active_requests.store(2, Ordering::SeqCst);

    assert_eq!(control.keda_pending_queue_size(), 3);
}

#[test]
fn keda_pending_signal_boosts_when_route_is_saturated() {
    let control = RouteExecutionControl::from_limits(0, 4);
    control.pending_waiters.store(3, Ordering::SeqCst);
    control.active_requests.store(4, Ordering::SeqCst);

    assert_eq!(control.keda_pending_queue_size(), 7);
}

#[test]
fn validate_integrity_config_normalizes_model_bindings() {
    let mut config = IntegrityConfig::default_sealed();
    let mut route = IntegrityRoute::user("/api/guest-ai");
    route.models = vec![IntegrityModelBinding {
        alias: " llama3 ".to_owned(),
        path: "  /models/llama3.gguf ".to_owned(),
        device: ModelDevice::Cuda,
        qos: RouteQos::Standard,
    }];
    config.routes = vec![route];

    let config = validate_integrity_config(config).expect("model bindings should validate");
    let route = config
        .sealed_route("/api/guest-ai")
        .expect("AI route should stay available");

    assert_eq!(
        route.models,
        vec![IntegrityModelBinding {
            alias: "llama3".to_owned(),
            path: "/models/llama3.gguf".to_owned(),
            device: ModelDevice::Cuda,
            qos: RouteQos::Standard,
        }]
    );
}

#[test]
fn validate_integrity_config_rejects_duplicate_model_aliases_across_routes() {
    let mut first = IntegrityRoute::user("/api/guest-ai");
    first.models = vec![IntegrityModelBinding {
        alias: "shared".to_owned(),
        path: "/models/shared-a.gguf".to_owned(),
        device: ModelDevice::Cpu,
        qos: RouteQos::Standard,
    }];
    let mut second = IntegrityRoute::user("/api/assistant");
    second.models = vec![IntegrityModelBinding {
        alias: "shared".to_owned(),
        path: "/models/shared-b.gguf".to_owned(),
        device: ModelDevice::Metal,
        qos: RouteQos::Standard,
    }];

    let error = validate_integrity_config(IntegrityConfig {
        routes: vec![first, second],
        ..IntegrityConfig::default_sealed()
    })
    .expect_err("duplicate model aliases should fail validation");

    assert!(error.to_string().contains("model alias `shared`"));
}

#[test]
fn validate_integrity_config_normalizes_custom_domains() {
    let mut config = IntegrityConfig::default_sealed();
    let mut route = IntegrityRoute::user("/api/guest-example");
    route.domains = vec![" API.Example.Test ".to_owned()];
    config.routes = vec![route];
    config.tls_address = Some(DEFAULT_TLS_ADDRESS.to_owned());

    let config = validate_integrity_config(config).expect("TLS domains should validate");
    let route = config
        .sealed_route("/api/guest-example")
        .expect("route should stay sealed");

    assert_eq!(route.domains, vec!["api.example.test".to_owned()]);
    assert_eq!(config.tls_address.as_deref(), Some(DEFAULT_TLS_ADDRESS));
}

#[test]
fn validate_integrity_config_rejects_duplicate_custom_domains() {
    let mut first = IntegrityRoute::user("/api/guest-example");
    first.domains = vec!["api.example.test".to_owned()];
    let mut second = IntegrityRoute::user("/api/guest-loop");
    second.domains = vec!["api.example.test".to_owned()];

    let error = validate_integrity_config(IntegrityConfig {
        tls_address: Some(DEFAULT_TLS_ADDRESS.to_owned()),
        routes: vec![first, second],
        ..IntegrityConfig::default_sealed()
    })
    .expect_err("duplicate domains should fail validation");

    assert!(error.to_string().contains("domain `api.example.test`"));
}

#[test]
fn validate_integrity_config_accepts_hibernating_ram_volume() {
    let mut config = IntegrityConfig::default_sealed();
    config.routes = vec![hibernating_ram_route(Path::new("/tmp/tachyon-ram-cache"))];

    let config = validate_integrity_config(config).expect("hibernating RAM volume should validate");
    let route = config
        .sealed_route("/api/guest-volume")
        .expect("route should remain sealed");

    assert_eq!(route.volumes[0].volume_type, VolumeType::Ram);
    assert_eq!(route.volumes[0].idle_timeout.as_deref(), Some("50ms"));
    assert_eq!(
        route.volumes[0].eviction_policy,
        Some(VolumeEvictionPolicy::Hibernate)
    );
}

#[test]
fn validate_integrity_config_accepts_volume_ttl_seconds() {
    let mut config = IntegrityConfig::default_sealed();
    config.routes = vec![ttl_managed_volume_route(Path::new("/tmp/tachyon-ttl"), 300)];

    let config = validate_integrity_config(config).expect("volume ttl should validate");
    let route = config
        .sealed_route("/api/guest-volume")
        .expect("route should remain sealed");

    assert_eq!(route.volumes[0].ttl_seconds, Some(300));
}

#[test]
fn collect_ttl_managed_paths_deduplicates_by_shortest_ttl() {
    let shared_dir = Path::new("/tmp/tachyon-ttl-shared");
    let config = IntegrityConfig {
        routes: vec![
            ttl_managed_volume_route(shared_dir, 300),
            ttl_managed_volume_route(shared_dir, 60),
        ],
        ..IntegrityConfig::default_sealed()
    };

    assert_eq!(
        collect_ttl_managed_paths(&config),
        vec![TtlManagedPath {
            host_path: PathBuf::from("/tmp/tachyon-ttl-shared"),
            ttl: Duration::from_secs(60),
        }]
    );
}

#[test]
fn storage_broker_serializes_concurrent_writes_against_shared_volume() {
    let volume_dir = unique_test_dir("tachyon-storage-broker");
    let route = storage_broker_test_route(&volume_dir);
    let broker = StorageBrokerManager::default();
    let start = Arc::new(std::sync::Barrier::new(9));

    let handles = (0..8)
        .map(|index| {
            let broker = broker.clone();
            let route = route.clone();
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                broker
                    .enqueue_write_for_route(
                        &route,
                        "/app/data/state.txt",
                        StorageWriteMode::Append,
                        format!("write-{index}\n").into_bytes(),
                    )
                    .expect("broker write should be accepted");
            })
        })
        .collect::<Vec<_>>();

    start.wait();
    for handle in handles {
        handle.join().expect("broker worker thread should complete");
    }

    assert!(
        broker.wait_for_volume_idle(&volume_dir, Duration::from_secs(5)),
        "broker queue should drain"
    );

    let contents = fs::read_to_string(volume_dir.join("state.txt"))
        .expect("brokered writes should reach the shared host volume");
    let mut lines = contents.lines().collect::<Vec<_>>();
    lines.sort_unstable();

    assert_eq!(
        lines,
        vec![
            "write-0", "write-1", "write-2", "write-3", "write-4", "write-5", "write-6", "write-7",
        ]
    );

    let _ = fs::remove_dir_all(volume_dir);
}

#[test]
fn storage_broker_emits_cdc_event_after_sync_enabled_write() {
    let volume_dir = unique_test_dir("tachyon-cdc-write");
    let store_path = unique_test_dir("tachyon-cdc-store").join("tachyon.db");
    let core_store = Arc::new(store::CoreStore::open(&store_path).expect("store should open"));
    let broker = StorageBrokerManager::new(Arc::clone(&core_store));
    let mut route = storage_broker_test_route(&volume_dir);

    broker
        .enqueue_write_for_route(
            &route,
            "/app/data/local.txt",
            StorageWriteMode::Overwrite,
            b"local-only".to_vec(),
        )
        .expect("non CDC write should be accepted");
    assert!(
        broker.wait_for_volume_idle(&volume_dir, Duration::from_secs(5)),
        "broker queue should drain"
    );
    assert!(
        core_store
            .peek_outbox(store::CoreStoreBucket::DataMutationOutbox, 10)
            .expect("outbox should be readable")
            .is_empty(),
        "non opt-in route should not emit CDC events"
    );

    route.sync_to_cloud = true;
    broker
        .enqueue_write_for_route(
            &route,
            "/app/data/state.txt",
            StorageWriteMode::Append,
            b"replicate-me".to_vec(),
        )
        .expect("CDC write should be accepted");
    assert!(
        broker.wait_for_volume_idle(&volume_dir, Duration::from_secs(5)),
        "broker queue should drain"
    );

    let events = core_store
        .peek_outbox(store::CoreStoreBucket::DataMutationOutbox, 10)
        .expect("outbox should be readable");
    assert_eq!(events.len(), 1);
    let payload: Value = serde_json::from_slice(&events[0].1).expect("CDC event should be JSON");
    assert_eq!(payload["event"], "tachyon.data.mutation");
    assert_eq!(payload["route_path"], route.path);
    assert_eq!(payload["resource"], "/app/data/state.txt");
    assert_eq!(payload["operation"], "append");
    assert_eq!(payload["value_bytes"], 12);
    assert_eq!(
        payload["value_hash"],
        format!("sha256:{}", hex::encode(Sha256::digest(b"replicate-me")))
    );

    let _ = fs::remove_dir_all(volume_dir);
    if let Some(parent) = store_path.parent() {
        let _ = fs::remove_dir_all(parent);
    }
}

#[tokio::test]
async fn storage_broker_enforces_signed_caller_scope_with_http_403() {
    let shared_dir = unique_test_dir("tachyon-zero-trust-broker");
    let tenant_a_dir = shared_dir.join("tenant-a");
    let tenant_b_dir = shared_dir.join("tenant-b");
    let config = validate_integrity_config(IntegrityConfig {
        routes: vec![
            scoped_volume_test_route("/api/tenant-a", &tenant_a_dir, "/data/tenant-a", true),
            scoped_volume_test_route("/api/tenant-b", &tenant_b_dir, "/data/tenant-b", true),
            storage_broker_test_route(&shared_dir),
        ],
        ..IntegrityConfig::default_sealed()
    })
    .expect("zero-trust broker config should validate");

    let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
    let broker = Arc::clone(&state.storage_broker);
    let caller_route = config
        .sealed_route("/api/tenant-a")
        .expect("tenant-a route should remain sealed");
    let token = state
        .host_identity
        .sign_route(caller_route)
        .expect("caller token should sign");
    let app = build_app(state);

    let forged = app
        .clone()
        .oneshot(
            Request::post("/system/storage-broker?path=/data/tenant-a/forged.txt")
                .header(TACHYON_IDENTITY_HEADER, "Bearer forged")
                .body(Body::from("forged"))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(forged.status(), StatusCode::FORBIDDEN);

    let accepted = app
        .clone()
        .oneshot(
            Request::post("/system/storage-broker?path=/data/tenant-a/state.txt")
                .header(TACHYON_IDENTITY_HEADER, format!("Bearer {token}"))
                .body(Body::from("allowed"))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    assert!(
        broker.wait_for_volume_idle(&tenant_a_dir, Duration::from_secs(5)),
        "tenant-a broker queue should drain"
    );
    assert_eq!(
        fs::read_to_string(tenant_a_dir.join("state.txt"))
            .expect("authorized write should reach tenant-a volume"),
        "allowed"
    );

    let denied = app
        .oneshot(
            Request::post("/system/storage-broker?path=/data/tenant-b/state.txt")
                .header(TACHYON_IDENTITY_HEADER, format!("Bearer {token}"))
                .body(Body::from("blocked"))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
    let denied_body = denied
        .into_body()
        .collect()
        .await
        .expect("response body should collect")
        .to_bytes();
    assert!(
        String::from_utf8_lossy(&denied_body).contains("cannot broker writes"),
        "unexpected denial body: {:?}",
        denied_body
    );
    assert!(
        !tenant_b_dir.join("state.txt").exists(),
        "out-of-scope write should not create tenant-b data"
    );

    let _ = fs::remove_dir_all(shared_dir);
}

#[tokio::test]
async fn volume_gc_tick_removes_stale_entries_from_short_lived_volume() {
    let volume_dir = unique_test_dir("tachyon-volume-gc");
    let stale_file = volume_dir.join("stale.txt");
    let stale_dir = volume_dir.join("stale-dir");
    fs::write(&stale_file, "stale").expect("stale file should be created");
    fs::create_dir_all(&stale_dir).expect("stale directory should be created");
    fs::write(stale_dir.join("nested.txt"), "stale").expect("nested file should be created");

    tokio::time::sleep(Duration::from_millis(1100)).await;

    let fresh_file = volume_dir.join("fresh.txt");
    fs::write(&fresh_file, "fresh").expect("fresh file should be created");

    let mut config = IntegrityConfig::default_sealed();
    config.routes = vec![ttl_managed_volume_route(&volume_dir, 1)];

    run_volume_gc_tick(Arc::new(build_test_runtime(config)))
        .await
        .expect("volume GC tick should complete");

    assert!(
        !stale_file.exists(),
        "stale file should be removed by the GC sweep"
    );
    assert!(
        !stale_dir.exists(),
        "stale directory should be removed by the GC sweep"
    );
    assert!(fresh_file.exists(), "fresh file should not be removed");

    let _ = fs::remove_dir_all(volume_dir);
}

#[tokio::test]
async fn hibernating_ram_volume_swaps_out_and_restores_state() {
    let volume_dir = unique_test_dir("tachyon-ram-hibernate");
    let route = hibernating_ram_route(&volume_dir);
    let broker = Arc::new(StorageBrokerManager::default());
    let volume_manager = VolumeManager::default();

    {
        let _leases = volume_manager
            .acquire_route_volumes(&route, Arc::clone(&broker))
            .await
            .expect("initial route volume acquisition should succeed");
        fs::write(volume_dir.join("state.txt"), "hibernated state")
            .expect("state file should be written");
    }

    let managed = volume_manager
        .managed_volume_for_route(&route.path, "/app/data")
        .expect("managed volume should be registered");

    for _ in 0..50 {
        if managed.lifecycle() == ManagedVolumeLifecycle::OnDisk {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    assert_eq!(managed.lifecycle(), ManagedVolumeLifecycle::OnDisk);
    assert!(
        broker
            .core_store
            .get(
                store::CoreStoreBucket::HibernationState,
                &managed_volume_id(&route.path, "/app/data"),
            )
            .expect("hibernation state lookup should succeed")
            .is_some(),
        "hibernation snapshot should be persisted in the core store"
    );
    assert!(
        !volume_dir.exists(),
        "active RAM volume directory should be released after hibernation"
    );

    let _restored = volume_manager
        .acquire_route_volumes(&route, Arc::clone(&broker))
        .await
        .expect("restoring hibernated volume should succeed");

    assert_eq!(managed.lifecycle(), ManagedVolumeLifecycle::Active);
    assert_eq!(
        fs::read_to_string(volume_dir.join("state.txt"))
            .expect("restored RAM volume should expose the original file"),
        "hibernated state"
    );

    let _ = fs::remove_dir_all(volume_dir);
}

#[test]
fn validate_integrity_config_rejects_zero_max_concurrency() {
    let mut config = IntegrityConfig::default_sealed();
    config.routes = vec![IntegrityRoute {
        path: "/api/guest-example".to_owned(),
        role: RouteRole::User,
        name: "guest-example".to_owned(),
        version: default_route_version(),
        dependencies: BTreeMap::new(),
        requires_credentials: Vec::new(),
        middleware: None,
        env: BTreeMap::new(),
        allowed_secrets: Vec::new(),
        targets: Vec::new(),
        resiliency: None,
        models: Vec::new(),
        domains: Vec::new(),
        min_instances: 0,
        max_concurrency: 0,
        volumes: Vec::new(),

        ..Default::default()
    }];

    let error =
        validate_integrity_config(config).expect_err("zero max_concurrency should fail validation");

    assert!(error
        .to_string()
        .contains("must set `max_concurrency` above zero"));
}

#[test]
fn validate_integrity_config_accepts_route_resiliency_policy() {
    let mut config = IntegrityConfig::default_sealed();
    let mut route = IntegrityRoute::user("/api/guest-example");
    route.resiliency = Some(ResiliencyConfig {
        timeout_ms: Some(500),
        retry_policy: Some(RetryPolicy {
            max_retries: 5,
            retry_on: vec![503, 502, 503],
        }),
    });
    config.routes = vec![route];

    let config =
        validate_integrity_config(config).expect("resiliency-enabled route should validate");
    let route = config
        .sealed_route("/api/guest-example")
        .expect("route should remain sealed");

    assert_eq!(
        route.resiliency,
        Some(ResiliencyConfig {
            timeout_ms: Some(500),
            retry_policy: Some(RetryPolicy {
                max_retries: 5,
                retry_on: vec![502, 503],
            }),
        })
    );
}

#[test]
fn validate_integrity_config_rejects_retry_policy_without_statuses() {
    let mut config = IntegrityConfig::default_sealed();
    let mut route = IntegrityRoute::user("/api/guest-example");
    route.resiliency = Some(ResiliencyConfig {
        timeout_ms: None,
        retry_policy: Some(RetryPolicy {
            max_retries: 2,
            retry_on: Vec::new(),
        }),
    });
    config.routes = vec![route];

    let error = validate_integrity_config(config)
        .expect_err("retry policy without retry_on statuses should fail validation");

    assert!(error
        .to_string()
        .contains("must configure at least one `resiliency.retry_policy.retry_on` status"));
}

#[cfg(not(feature = "resiliency"))]
#[tokio::test]
async fn route_resiliency_config_is_overhead_free_when_feature_is_disabled() {
    let config = validate_integrity_config(IntegrityConfig {
        routes: vec![resiliency_test_route(Some(ResiliencyConfig {
            timeout_ms: Some(500),
            retry_policy: Some(RetryPolicy {
                max_retries: 5,
                retry_on: vec![503],
            }),
        }))],
        ..IntegrityConfig::default_sealed()
    })
    .expect("resiliency route should validate without the feature");
    let app = build_app(build_test_state(config, telemetry::init_test_telemetry()));

    let response = app
        .oneshot(
            Request::post("/api/guest-flaky")
                .body(Body::from("force-fail"))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[cfg(feature = "resiliency")]
#[tokio::test]
async fn route_resiliency_timeout_applies_to_guest_execution() {
    let config = validate_integrity_config(IntegrityConfig {
        routes: vec![resiliency_test_route(Some(ResiliencyConfig {
            timeout_ms: Some(50),
            retry_policy: None,
        }))],
        ..IntegrityConfig::default_sealed()
    })
    .expect("resiliency route should validate");
    let app = build_app(build_test_state(config, telemetry::init_test_telemetry()));

    let response = app
        .oneshot(
            Request::post("/api/guest-flaky")
                .body(Body::from("sleep:2000"))
                .expect("request should build"),
        )
        .await
        .expect("request should complete");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body should collect")
        .to_bytes();

    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert!(String::from_utf8_lossy(&body).contains("timed out after 50ms"));
}

#[test]
fn embedded_integrity_payload_is_a_valid_runtime_config() {
    let config = serde_json::from_str::<IntegrityConfig>(EMBEDDED_CONFIG_PAYLOAD)
        .expect("embedded payload should deserialize into an integrity config");
    let config = validate_integrity_config(config).expect("embedded config should validate");

    assert_eq!(config.guest_fuel_budget, DEFAULT_GUEST_FUEL_BUDGET);
    assert_eq!(
        config
            .sealed_route("/metrics")
            .expect("embedded config should seal the system metrics route")
            .role,
        RouteRole::System
    );
    assert_eq!(
        config
            .sealed_route("/api/guest-example")
            .expect("embedded config should seal the example route")
            .allowed_secrets,
        vec!["DB_PASS".to_owned()]
    );
    assert_eq!(
        config
            .sealed_route("/api/guest-example")
            .expect("embedded config should seal the example route")
            .min_instances,
        0
    );
    assert_eq!(
        config
            .sealed_route("/api/guest-example")
            .expect("embedded config should seal the example route")
            .max_concurrency,
        DEFAULT_ROUTE_MAX_CONCURRENCY
    );
    assert!(config.sealed_route("/api/guest-example").is_some());
    assert!(config.sealed_route("/api/guest-loop").is_some());
    assert!(config.sealed_route("/api/guest-csharp").is_some());
    assert!(config.sealed_route("/api/guest-java").is_some());
}

#[test]
fn embedded_integrity_payload_allows_legacy_service_resource_alias() {
    let config = serde_json::from_str::<IntegrityConfig>(EMBEDDED_CONFIG_PAYLOAD)
        .expect("embedded payload should deserialize into an integrity config");
    let config = validate_integrity_config(config).expect("embedded config should validate");
    let route_registry = RouteRegistry::build(&config).expect("route registry should build");
    let caller_route = config
        .sealed_route("/api/guest-call-legacy")
        .expect("legacy route should remain sealed");

    assert_eq!(
        resolve_outbound_http_target(
            &config,
            &route_registry,
            caller_route,
            &reqwest::Method::GET,
            "http://mesh/legacy-service/ping",
        )
        .expect("legacy service alias should resolve"),
        ResolvedOutboundTarget {
            url: "http://legacy-service:8081/ping".to_owned(),
            kind: OutboundTargetKind::External,
        }
    );
}

#[test]
fn guest_module_candidates_cover_release_and_container_paths() {
    let candidates = guest_module_candidate_paths("guest-example")
        .into_iter()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>();

    assert!(candidates.iter().any(|path| {
        path.ends_with("/target/wasm32-wasip2/release/guest_example.wasm")
            || path == "target/wasm32-wasip2/release/guest_example.wasm"
    }));
    assert!(candidates.iter().any(|path| {
        path.ends_with("/target/wasm32-wasip1/release/guest_example.wasm")
            || path == "target/wasm32-wasip1/release/guest_example.wasm"
    }));
    assert!(candidates
        .iter()
        .any(|path| path.ends_with("guest-modules/guest_example.wasm")));
}

#[test]
fn guest_module_candidates_normalize_hyphenated_names_to_underscores() {
    let candidates = guest_module_candidate_paths("guest-csharp")
        .into_iter()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect::<Vec<_>>();

    assert!(candidates
        .iter()
        .any(|path| path.ends_with("guest-modules/guest_csharp.wasm")));
}

#[test]
fn guest_ai_is_gated_behind_ai_inference_feature() {
    assert!(requires_ai_inference_feature("guest-ai"));
    assert!(!requires_ai_inference_feature("guest-example"));
}

#[test]
fn legacy_guest_program_name_is_a_guest_visible_relative_path() {
    let program_name = legacy_guest_program_name(Path::new("/app/guest-modules/guest_csharp.wasm"));

    assert_eq!(program_name, "./guest_csharp.wasm");
}

#[test]
fn resolve_function_name_supports_hyphenated_guest_routes() {
    assert_eq!(
        resolve_function_name("/api/guest-java"),
        Some("guest-java".to_owned())
    );
}

#[test]
fn classify_resource_limit_detects_fuel_traps() {
    let error: wasmtime::Error = Trap::OutOfFuel.into();

    assert_eq!(
        classify_resource_limit(&error),
        Some(ResourceLimitKind::Fuel)
    );
}

#[test]
fn zero_exit_status_from_command_guest_is_treated_as_success() {
    let result = handle_guest_entrypoint_result("_start", Err(I32Exit(0).into()));

    assert!(result.is_ok());
}

#[test]
fn nonzero_exit_status_from_start_guest_is_preserved_as_success() {
    let result = handle_guest_entrypoint_result("_start", Err(I32Exit(1).into()));

    assert!(result.is_ok());
}

#[test]
fn nonzero_exit_status_from_faas_entry_remains_an_error() {
    let error = handle_guest_entrypoint_result("faas_entry", Err(I32Exit(1).into()))
        .expect_err("non-zero faas_entry exit should fail");

    match error {
        ExecutionError::Internal(message) => {
            assert!(message.contains("Exited with i32 exit status 1"));
        }
        other => unreachable!("unexpected error variant: {other:?}"),
    }
}
