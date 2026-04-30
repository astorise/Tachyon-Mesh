use super::support_and_cache::*;
use crate::*;

#[test]
fn validate_integrity_config_normalizes_routes() {
    let mut config = IntegrityConfig::default_sealed();
    config.routes = vec![
        IntegrityRoute::user("api/guest-example"),
        IntegrityRoute::user("/api/guest-malicious"),
        IntegrityRoute::system("/metrics/"),
    ];

    let config = validate_integrity_config(config).expect("config should validate");

    assert_eq!(
        config.routes,
        vec![
            IntegrityRoute::user("/api/guest-example"),
            IntegrityRoute::user("/api/guest-malicious"),
            IntegrityRoute::system("/metrics"),
        ]
    );
}

#[test]
fn validate_integrity_config_accepts_batch_targets_without_routes() {
    let temp_dir = unique_test_dir("batch-targets");
    let cache_dir = temp_dir.join("cache");
    fs::create_dir_all(&cache_dir).expect("cache directory should be created");

    let mut config = IntegrityConfig::default_sealed();
    config.routes.clear();
    config.batch_targets = vec![gc_batch_target(&cache_dir, 60)];

    let config = validate_integrity_config(config).expect("batch-only config should validate");

    assert!(config.routes.is_empty());
    assert_eq!(config.batch_targets.len(), 1);
    assert_eq!(config.batch_targets[0].name, "gc-job");
}

#[test]
fn validate_integrity_config_rejects_duplicate_routes() {
    let mut config = IntegrityConfig::default_sealed();
    config.routes = vec![
        IntegrityRoute::user("/metrics"),
        IntegrityRoute::system("/metrics/"),
    ];

    let error = validate_integrity_config(config)
        .expect_err("duplicate normalized routes should fail validation");

    assert!(error.to_string().contains("defined more than once"));
}

#[test]
fn validate_integrity_config_defaults_route_scaling_when_scaling_fields_are_omitted() {
    let config = serde_json::from_str::<IntegrityConfig>(
            r#"{
                "host_address":"0.0.0.0:8080",
                "max_stdout_bytes":65536,
                "guest_fuel_budget":500000000,
                "guest_memory_limit_bytes":52428800,
                "resource_limit_response":"Execution trapped: Resource limit exceeded",
                "routes":[{"path":"/api/guest-example","role":"user","version":"0.0.0","dependencies":{}}]
            }"#,
        )
        .expect("payload should deserialize");
    let config = validate_integrity_config(config).expect("payload should validate");
    let route = config
        .sealed_route("/api/guest-example")
        .expect("route should remain sealed");

    assert_eq!(route.name, "guest-example");
    assert_eq!(route.version, DEFAULT_ROUTE_VERSION);
    assert!(route.dependencies.is_empty());
    assert!(route.requires_credentials.is_empty());
    assert!(route.middleware.is_none());
    assert_eq!(route.min_instances, 0);
    assert_eq!(route.max_concurrency, DEFAULT_ROUTE_MAX_CONCURRENCY);
    assert!(route.volumes.is_empty());
}

#[test]
fn verify_integrity_payload_returns_schema_violation_when_version_is_missing() {
    let payload = r#"{
            "host_address":"0.0.0.0:8080",
            "max_stdout_bytes":65536,
            "guest_fuel_budget":500000000,
            "guest_memory_limit_bytes":52428800,
            "resource_limit_response":"Execution trapped: Resource limit exceeded",
            "routes":[{"path":"/api/guest-example","role":"user","dependencies":{}}]
        }"#;
    let signing_key = SigningKey::from_bytes(&[21_u8; 32]);
    let signature = signing_key.sign(&Sha256::digest(payload.as_bytes()));
    let error = verify_integrity_payload(
        payload,
        &hex::encode(signing_key.verifying_key().to_bytes()),
        &hex::encode(signature.to_bytes()),
        "test payload",
    )
    .expect_err("payload missing version should fail strict schema validation");

    let message = format!("{error:#}");
    assert!(message.contains(ERR_INTEGRITY_SCHEMA_VIOLATION));
}

#[test]
fn build_test_state_prewarms_min_instances() {
    let mut route = IntegrityRoute::user(DEFAULT_ROUTE);
    route.min_instances = 2;
    route.max_concurrency = 4;

    let state = build_test_state(
        IntegrityConfig {
            routes: vec![route.clone()],
            ..IntegrityConfig::default_sealed()
        },
        telemetry::init_test_telemetry(),
    );
    let runtime = state.runtime.load_full();
    let control = runtime
        .concurrency_limits
        .get(&route.path)
        .expect("route should have an execution control");

    assert_eq!(control.prewarmed_instances(), 2);
}

#[test]
fn validate_integrity_config_rejects_invalid_telemetry_sample_rate() {
    let error = validate_integrity_config(IntegrityConfig {
        telemetry_sample_rate: 1.5,
        ..IntegrityConfig::default_sealed()
    })
    .expect_err("sample rates above one should fail validation");

    assert!(error.to_string().contains("`telemetry_sample_rate`"));
}

#[test]
fn validate_integrity_config_rejects_duplicate_tcp_layer4_ports() {
    let error = validate_integrity_config(IntegrityConfig {
        layer4: IntegrityLayer4Config {
            tcp: vec![
                IntegrityTcpBinding {
                    port: 2222,
                    target: "guest-tcp-echo".to_owned(),
                },
                IntegrityTcpBinding {
                    port: 2222,
                    target: "guest-tcp-echo".to_owned(),
                },
            ],
            udp: Vec::new(),
        },
        routes: vec![tcp_echo_test_route(1)],
        ..IntegrityConfig::default_sealed()
    })
    .expect_err("duplicate TCP Layer 4 ports should fail validation");

    assert!(error.to_string().contains("Layer 4 port `2222`"));
}

#[test]
fn validate_integrity_config_rejects_duplicate_udp_layer4_ports() {
    let error = validate_integrity_config(IntegrityConfig {
        layer4: IntegrityLayer4Config {
            tcp: Vec::new(),
            udp: vec![
                IntegrityUdpBinding {
                    port: 5353,
                    target: "guest-udp-echo".to_owned(),
                },
                IntegrityUdpBinding {
                    port: 5353,
                    target: "guest-udp-echo".to_owned(),
                },
            ],
        },
        routes: vec![udp_echo_test_route(1)],
        ..IntegrityConfig::default_sealed()
    })
    .expect_err("duplicate UDP Layer 4 ports should fail validation");

    assert!(error.to_string().contains("Layer 4 port `5353`"));
}

#[test]
fn validate_integrity_config_rejects_unsatisfied_semver_dependencies() {
    let mut config = IntegrityConfig::default_sealed();
    config.routes = vec![
        dependency_route("/api/faas-a", "faas-a", "2.0.0", &[("faas-b", "^2.0")]),
        versioned_route("/api/faas-b-v1", "faas-b", "1.5.0"),
    ];

    let error = validate_integrity_config(config)
        .expect_err("unsatisfied dependency graph should fail validation");

    assert!(error.to_string().contains("requires faas-b matching ^2.0"));
}

#[test]
fn validate_integrity_config_rejects_resource_names_that_conflict_with_routes() {
    let mut config = IntegrityConfig::default_sealed();
    config.routes = vec![versioned_route("/api/checkout", "checkout", "1.0.0")];
    config.resources = BTreeMap::from([(
        "checkout".to_owned(),
        IntegrityResource::External {
            target: "https://api.example.com/v1".to_owned(),
            allowed_methods: vec!["GET".to_owned()],
        },
    )]);

    let error = validate_integrity_config(config)
        .expect_err("resource names that shadow routes should fail validation");

    assert!(error
        .to_string()
        .contains("conflicts with a sealed route name"));
}

#[test]
fn validate_integrity_config_rejects_external_resources_without_allowed_methods() {
    let mut config = IntegrityConfig::default_sealed();
    config.routes = vec![versioned_route("/api/checkout", "checkout", "1.0.0")];
    config.resources = BTreeMap::from([(
        "payment-gateway".to_owned(),
        IntegrityResource::External {
            target: "https://api.example.com/v1".to_owned(),
            allowed_methods: Vec::new(),
        },
    )]);

    let error = validate_integrity_config(config)
        .expect_err("external resources must declare an allow-list");

    assert!(error
        .to_string()
        .contains("must declare at least one allowed HTTP method"));
}

#[test]
fn validate_integrity_config_accepts_http_cluster_local_external_resource() {
    let mut config = IntegrityConfig::default_sealed();
    config.routes = vec![versioned_route("/api/checkout", "checkout", "1.0.0")];
    config.resources = BTreeMap::from([(
        "legacy-service".to_owned(),
        IntegrityResource::External {
            target: "http://legacy-service:8081".to_owned(),
            allowed_methods: vec!["GET".to_owned()],
        },
    )]);

    let config =
        validate_integrity_config(config).expect("cluster-local HTTP resource should validate");

    assert_eq!(
        config.resources.get("legacy-service"),
        Some(&IntegrityResource::External {
            target: "http://legacy-service:8081/".to_owned(),
            allowed_methods: vec!["GET".to_owned()],
        })
    );
}

#[test]
fn validate_integrity_config_rejects_public_http_external_resource() {
    let mut config = IntegrityConfig::default_sealed();
    config.routes = vec![versioned_route("/api/checkout", "checkout", "1.0.0")];
    config.resources = BTreeMap::from([(
        "payment-gateway".to_owned(),
        IntegrityResource::External {
            target: "http://api.example.com/v1".to_owned(),
            allowed_methods: vec!["GET".to_owned()],
        },
    )]);

    let error = validate_integrity_config(config)
        .expect_err("public HTTP external resources should be rejected");

    assert!(error.to_string().contains(
        "must use HTTPS unless it points at localhost for tests or a cluster-local service"
    ));
}

#[test]
fn validate_integrity_config_accepts_system_connector_dependencies() {
    let host_address = DEFAULT_HOST_ADDRESS
        .parse::<SocketAddr>()
        .expect("default host address should parse");
    let config = validate_integrity_config(sqs_connector_test_config(
        host_address,
        "http://queue.local/mock".to_owned(),
        "/api/connector-target",
        "guest-example",
    ))
    .expect("system connector dependencies should validate");

    let connector = config
        .sealed_route("/system/sqs-connector")
        .expect("connector route should remain sealed");
    let expected_requirement = VersionReq::parse(&default_route_version())
        .expect("default route version should normalize as a requirement")
        .to_string();

    assert_eq!(
        connector.dependencies.get("connector-target"),
        Some(&expected_requirement)
    );
    assert_eq!(
        connector.env.get("QUEUE_URL"),
        Some(&"http://queue.local/mock".to_owned())
    );
    assert_eq!(
        connector.env.get("TARGET_ROUTE"),
        Some(&"/api/connector-target".to_owned())
    );
}

#[test]
fn validate_integrity_config_rejects_missing_delegated_credentials() {
    let mut config = IntegrityConfig::default_sealed();
    let faas_a = dependency_route("/api/faas-a", "faas-a", "2.0.0", &[("faas-b", "^2.0")]);
    let mut faas_b = versioned_route("/api/faas-b-v2", "faas-b", "2.1.0");
    faas_b.requires_credentials = vec!["c2".to_owned()];
    config.routes = vec![faas_a, faas_b];

    let error = validate_integrity_config(config)
        .expect_err("missing delegated credentials should fail validation");

    assert!(error.to_string().contains("Credential delegation failed"));
    assert!(error.to_string().contains("c2"));
}

#[test]
fn validate_integrity_config_accepts_satisfied_delegated_credentials() {
    let mut config = IntegrityConfig::default_sealed();
    let mut faas_a = dependency_route("/api/faas-a", "faas-a", "2.0.0", &[("faas-b", "^2.0")]);
    faas_a.requires_credentials = vec!["c2".to_owned()];
    let mut faas_b = versioned_route("/api/faas-b-v2", "faas-b", "2.1.0");
    faas_b.requires_credentials = vec!["c2".to_owned()];
    config.routes = vec![faas_a, faas_b];

    let config = validate_integrity_config(config)
        .expect("delegated credentials should satisfy dependency validation");
    let route = config
        .sealed_route("/api/faas-a")
        .expect("caller route should remain sealed");

    assert_eq!(route.requires_credentials, vec!["c2".to_owned()]);
}

#[test]
fn validate_integrity_config_rejects_unknown_middleware_route() {
    let mut config = IntegrityConfig::default_sealed();
    let mut protected = IntegrityRoute::user(DEFAULT_ROUTE);
    protected.middleware = Some("missing-auth".to_owned());
    config.routes = vec![protected];

    let error = validate_integrity_config(config)
        .expect_err("unknown middleware route should fail validation");

    assert!(error
        .to_string()
        .contains("route middleware `missing-auth`"));
}

#[test]
fn middleware_routes_short_circuit_non_ok_responses_and_allow_ok_responses() {
    fn simulate_middleware_chain(
        runtime: &RuntimeState,
        route: &IntegrityRoute,
        responses: &HashMap<String, GuestHttpResponse>,
        visited: &mut Vec<String>,
    ) -> GuestHttpResponse {
        if let Some(middleware_name) = route.middleware.as_deref() {
            let middleware = runtime
                .route_registry
                .resolve_named_route(middleware_name)
                .expect("middleware route should resolve");
            let middleware_route = runtime
                .config
                .sealed_route(&middleware.path)
                .expect("middleware route should stay sealed");
            visited.push(middleware_route.path.clone());
            let middleware_response = responses
                .get(&middleware_route.path)
                .expect("middleware response should be defined")
                .clone();
            if middleware_response.status != StatusCode::OK {
                return middleware_response;
            }
        }

        visited.push(route.path.clone());
        responses
            .get(&route.path)
            .expect("main route response should be defined")
            .clone()
    }

    let mut protected_allow = targeted_route(
        "/api/protected-allow",
        vec![weighted_target("guest-example", 100)],
    );
    protected_allow.name = "protected-allow".to_owned();
    protected_allow.middleware = Some("allow-middleware".to_owned());

    let mut protected_deny = targeted_route(
        "/api/protected-deny",
        vec![weighted_target("guest-example", 100)],
    );
    protected_deny.name = "protected-deny".to_owned();
    protected_deny.middleware = Some("deny-middleware".to_owned());

    let mut allow_middleware = IntegrityRoute::user("/api/allow-middleware");
    allow_middleware.name = "allow-middleware".to_owned();

    let mut deny_middleware = IntegrityRoute::user("/api/deny-middleware");
    deny_middleware.name = "deny-middleware".to_owned();

    let config = IntegrityConfig {
        routes: vec![
            protected_allow.clone(),
            protected_deny.clone(),
            allow_middleware,
            deny_middleware,
        ],
        ..IntegrityConfig::default_sealed()
    };
    let runtime =
        build_test_runtime(validate_integrity_config(config).expect("test config should validate"));

    let allow_route = runtime
        .config
        .sealed_route("/api/protected-allow")
        .expect("allow route should stay sealed");
    let deny_route = runtime
        .config
        .sealed_route("/api/protected-deny")
        .expect("deny route should stay sealed");

    let mut responses = HashMap::new();
    responses.insert(
        "/api/allow-middleware".to_owned(),
        GuestHttpResponse::new(StatusCode::OK, "middleware allowed"),
    );
    responses.insert(
        "/api/protected-allow".to_owned(),
        GuestHttpResponse::new(
            StatusCode::OK,
            Bytes::from(expected_guest_example_body(
                "FaaS received an empty payload",
            )),
        ),
    );
    responses.insert(
        "/api/deny-middleware".to_owned(),
        GuestHttpResponse::new(StatusCode::FORBIDDEN, "forbidden"),
    );
    responses.insert(
        "/api/protected-deny".to_owned(),
        GuestHttpResponse::new(StatusCode::OK, "main route should not execute"),
    );

    let mut allow_visited = Vec::new();
    let allow_response =
        simulate_middleware_chain(&runtime, allow_route, &responses, &mut allow_visited);
    assert_eq!(
        allow_visited,
        vec![
            "/api/allow-middleware".to_owned(),
            "/api/protected-allow".to_owned()
        ]
    );
    assert_eq!(allow_response.status, StatusCode::OK);
    assert_eq!(
        allow_response.body,
        Bytes::from(expected_guest_example_body(
            "FaaS received an empty payload"
        ))
    );

    let mut deny_visited = Vec::new();
    let deny_response =
        simulate_middleware_chain(&runtime, deny_route, &responses, &mut deny_visited);
    assert_eq!(deny_visited, vec!["/api/deny-middleware".to_owned()]);
    assert_eq!(deny_response.status, StatusCode::FORBIDDEN);
    assert_eq!(deny_response.body, Bytes::from("forbidden"));
}

#[test]
fn validate_integrity_config_normalizes_route_volumes() {
    let mut config = IntegrityConfig::default_sealed();
    config.routes = vec![IntegrityRoute {
        path: "/api/guest-volume".to_owned(),
        role: RouteRole::User,
        name: "guest-volume".to_owned(),
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
        max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
        volumes: vec![IntegrityVolume {
            volume_type: VolumeType::Host,
            host_path: "  /tmp/tachyon_data  ".to_owned(),
            guest_path: "/app/data/".to_owned(),
            readonly: true,
            ttl_seconds: None,
            idle_timeout: None,
            eviction_policy: None,

            ..Default::default()
        }],

        ..Default::default()
    }];

    let config = validate_integrity_config(config).expect("volume config should validate");
    let route = config
        .sealed_route("/api/guest-volume")
        .expect("route should remain sealed");

    assert_eq!(
        route.volumes,
        vec![IntegrityVolume {
            volume_type: VolumeType::Host,
            host_path: "/tmp/tachyon_data".to_owned(),
            guest_path: "/app/data".to_owned(),
            readonly: true,
            ttl_seconds: None,
            idle_timeout: None,
            eviction_policy: None,

            ..Default::default()
        }]
    );
}

#[test]
fn validate_integrity_config_preserves_encrypted_volume_flag() {
    let mut config = IntegrityConfig::default_sealed();
    config.routes = vec![IntegrityRoute {
        path: "/system/tde-consumer".to_owned(),
        role: RouteRole::System,
        name: "system-faas-logger".to_owned(),
        version: default_route_version(),
        dependencies: BTreeMap::new(),
        volumes: vec![IntegrityVolume {
            volume_type: VolumeType::Host,
            host_path: "/tmp/tachyon_sensitive".to_owned(),
            guest_path: "/secure".to_owned(),
            readonly: false,
            encrypted: true,
            ..Default::default()
        }],
        ..Default::default()
    }];

    let config =
        validate_integrity_config(config).expect("encrypted volume config should validate");
    let volume = &config
        .sealed_route("/system/tde-consumer")
        .expect("route should remain sealed")
        .volumes[0];

    assert!(volume.encrypted);
    assert_eq!(
        encrypted_volume_host_path(&volume.host_path),
        PathBuf::from("/tmp/tachyon_sensitive").join(".tachyon-tde")
    );
}

#[test]
fn encrypted_volume_seal_hides_plaintext_and_prepare_restores_it() {
    let volume_dir = unique_test_dir("tachyon-tde-volume");
    let mut route = storage_broker_test_route(&volume_dir);
    route.volumes[0].encrypted = true;
    let encrypted_root = encrypted_volume_host_path(&route.volumes[0].host_path);
    fs::create_dir_all(&encrypted_root).expect("encrypted root should exist");
    let file_path = encrypted_root.join("state.txt");
    fs::write(&file_path, b"patient-record: secret").expect("plaintext should be written");

    seal_encrypted_route_volumes(&route).expect("volume should seal");
    let sealed = fs::read(&file_path).expect("sealed file should be readable");
    assert!(sealed.starts_with(TDE_FILE_MAGIC));
    assert!(!String::from_utf8_lossy(&sealed).contains("patient-record"));

    prepare_encrypted_route_volumes(&route).expect("volume should prepare");
    assert_eq!(
        fs::read(&file_path).expect("prepared file should be readable"),
        b"patient-record: secret"
    );

    let _ = fs::remove_dir_all(volume_dir);
}

#[test]
fn lora_training_job_exports_adapter_with_finops_metadata() {
    let broker_dir = unique_test_dir("tachyon-lora-train");
    std::env::set_var(MODEL_BROKER_DIR_ENV, &broker_dir);
    let statuses = Arc::new(Mutex::new(HashMap::new()));
    let job = LoraTrainingJob {
        id: "lora-test".to_owned(),
        tenant_id: "tenant-a".to_owned(),
        base_model: "llama3".to_owned(),
        dataset_volume: "training-data".to_owned(),
        dataset_path: "/datasets/a.jsonl".to_owned(),
        dataset_split: Some("train[:90%]".to_owned()),
        rank: 8,
        max_steps: 2,
        seed: Some(7),
    };

    let adapter_path =
        execute_lora_training_job(&job, &statuses).expect("training job should export");
    let payload = fs::read_to_string(&adapter_path).expect("adapter artifact should exist");
    let value: Value = serde_json::from_str(&payload).expect("adapter artifact should be JSON");

    assert_eq!(value["tenant_id"], "tenant-a");
    assert_eq!(value["base_model"], "llama3");
    assert_eq!(value["finops"]["cpu_fallback"], true);
    assert_eq!(value["finops"]["ram_spillover"], true);
    assert!(adapter_path.ends_with(".safetensors"));

    std::env::remove_var(MODEL_BROKER_DIR_ENV);
    let _ = fs::remove_dir_all(broker_dir);
}

#[test]
fn validate_integrity_config_rejects_tee_route_without_backend() {
    let mut route = IntegrityRoute::user("/api/guest-example");
    route.requires_tee = true;
    let config = IntegrityConfig {
        routes: vec![route],
        tee_backend: None,
        ..IntegrityConfig::default_sealed()
    };

    let error =
        validate_integrity_config(config).expect_err("TEE routes must require an explicit backend");

    assert!(error
        .to_string()
        .contains("routes with `requires_tee: true` require `tee_backend`"));
}

#[test]
fn validate_integrity_config_accepts_tee_route_with_backend() {
    let mut route = IntegrityRoute::user("/api/guest-example");
    route.requires_tee = true;
    let config = IntegrityConfig {
        routes: vec![route],
        tee_backend: Some(TeeBackendConfig::LocalEnclave),
        ..IntegrityConfig::default_sealed()
    };

    let config = validate_integrity_config(config).expect("TEE backend should validate");
    assert!(
        config
            .sealed_route("/api/guest-example")
            .expect("TEE route should remain sealed")
            .requires_tee
    );
}

#[test]
fn validate_integrity_config_rejects_writable_user_route_volumes() {
    let mut config = IntegrityConfig::default_sealed();
    config.routes = vec![IntegrityRoute {
        path: "/api/guest-volume".to_owned(),
        role: RouteRole::User,
        name: "guest-volume".to_owned(),
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
        max_concurrency: DEFAULT_ROUTE_MAX_CONCURRENCY,
        volumes: vec![IntegrityVolume {
            volume_type: VolumeType::Host,
            host_path: "/tmp/tachyon_data".to_owned(),
            guest_path: "/app/data".to_owned(),
            readonly: false,
            ttl_seconds: None,
            idle_timeout: None,
            eviction_policy: None,

            ..Default::default()
        }],

        ..Default::default()
    }];

    let error = validate_integrity_config(config)
        .expect_err("writable user volumes should fail validation");

    assert!(error
        .to_string()
        .contains("cannot request writable direct host mounts"));
}
