    #[tokio::test]
    async fn websocket_upgrade_is_rejected_without_feature_flag() {
        let route = targeted_route("/ws/echo", vec![websocket_target("guest-websocket-echo")]);
        let app = build_app(build_test_state(
            IntegrityConfig {
                routes: vec![route],
                ..IntegrityConfig::default_sealed()
            },
            telemetry::init_test_telemetry(),
        ));

        let response = app
            .oneshot(
                Request::get("/ws/echo")
                    .header("connection", "Upgrade")
                    .header("upgrade", "websocket")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should complete");

        assert_eq!(response.status(), StatusCode::NOT_IMPLEMENTED);
    }

    #[cfg(feature = "websockets")]
    #[tokio::test(flavor = "multi_thread")]
    async fn websocket_route_upgrades_and_echoes_frames() {
        use futures_util::{SinkExt, StreamExt};
        use std::time::Duration;
        use tokio_tungstenite::tungstenite::Message;

        let route = targeted_route("/ws/echo", vec![websocket_target("guest-websocket-echo")]);
        let config = validate_integrity_config(IntegrityConfig {
            routes: vec![route],
            ..IntegrityConfig::default_sealed()
        })
        .expect("WebSocket route config should validate");
        let app = build_app(build_test_state(config, telemetry::init_test_telemetry()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("WebSocket test listener should bind");
        let address = listener
            .local_addr()
            .expect("WebSocket test listener should expose an address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app.into_make_service())
                .await
                .expect("WebSocket test server should stay up");
        });

        let url = format!("ws://{address}/ws/echo");
        let (mut client, _) = tokio_tungstenite::connect_async(&url)
            .await
            .expect("WebSocket client should connect");

        client
            .send(Message::Text("hello".into()))
            .await
            .expect("WebSocket client should send text frame");
        let text_frame = client
            .next()
            .await
            .expect("WebSocket server should respond")
            .expect("WebSocket frame should be valid");
        assert!(matches!(text_frame, Message::Text(text) if text == "hello"));

        client
            .send(Message::Binary(vec![1_u8, 2, 3].into()))
            .await
            .expect("WebSocket client should send binary frame");
        let binary_frame = client
            .next()
            .await
            .expect("WebSocket server should respond to binary frame")
            .expect("WebSocket frame should be valid");
        assert!(matches!(binary_frame, Message::Binary(bytes) if bytes.as_ref() == [1_u8, 2, 3]));

        client
            .close(None)
            .await
            .expect("WebSocket client should initiate close");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match client.next().await {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => continue,
                    Some(Err(error)) => {
                        unreachable!("WebSocket close should not error: {error}");
                        break;
                    }
                }
            }
        })
        .await
        .expect("WebSocket guest should shut down after close");

        server.abort();
        let _ = server.await;
    }

    #[cfg(feature = "secrets-vault")]
    #[tokio::test]
    async fn router_denies_secret_lookup_without_sealed_grant() {
        let app = build_app(build_test_state(
            IntegrityConfig {
                routes: vec![IntegrityRoute::user("/api/guest-example")],
                ..IntegrityConfig::default_sealed()
            },
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
            "FaaS received an empty payload | env: missing | secret: permission-denied"
        );
    }

    #[test]
    fn guest_resource_limiter_rejects_memory_growth_past_ceiling() {
        let config = IntegrityConfig::default_sealed();
        let mut limiter = GuestResourceLimiter::new(config.guest_memory_limit_bytes);
        let error = limiter
            .memory_growing(
                config.guest_memory_limit_bytes,
                config.guest_memory_limit_bytes + 64 * 1024,
                None,
            )
            .expect_err("growth past the quota should fail");

        assert_eq!(
            error
                .downcast_ref::<ResourceLimitTrap>()
                .map(|error| error.kind),
            Some(ResourceLimitKind::Memory)
        );
    }

    #[test]
    fn extract_mesh_fetch_url_recognizes_bridge_command() {
        let stdout = Bytes::from("MESH_FETCH:http://mesh/legacy-service/ping\n");

        assert_eq!(
            extract_mesh_fetch_url(&stdout),
            Some("http://mesh/legacy-service/ping")
        );
    }

    #[test]
    fn extract_mesh_fetch_url_ignores_regular_guest_output() {
        let stdout = Bytes::from("FaaS received: Hello Lean FaaS!\n");

        assert_eq!(extract_mesh_fetch_url(&stdout), None);
    }

    #[test]
    fn select_route_module_prefers_matching_header_targets() {
        let route = targeted_route(
            "/api/checkout",
            vec![
                header_target("guest-loop", COHORT_HEADER, "beta"),
                weighted_target("guest-example", 100),
            ],
        );
        let mut headers = HeaderMap::new();
        headers.insert(COHORT_HEADER, HeaderValue::from_static("beta"));

        assert_eq!(
            select_route_target_with_roll(&route, &headers, Some(42))
                .expect("header-target route should resolve"),
            test_selected_target("guest-loop", false)
        );
    }

    #[test]
    fn select_route_module_uses_weighted_rollout_without_matching_headers() {
        let route = targeted_route(
            "/api/checkout",
            vec![
                weighted_target("guest-example", 90),
                weighted_target("guest-loop", 10),
            ],
        );

        assert_eq!(
            select_route_target_with_roll(&route, &HeaderMap::new(), Some(0))
                .expect("weighted route should resolve"),
            test_selected_target("guest-example", false)
        );
        assert_eq!(
            select_route_target_with_roll(&route, &HeaderMap::new(), Some(89))
                .expect("weighted route should resolve"),
            test_selected_target("guest-example", false)
        );
        assert_eq!(
            select_route_target_with_roll(&route, &HeaderMap::new(), Some(90))
                .expect("weighted route should resolve"),
            test_selected_target("guest-loop", false)
        );
    }

    #[test]
    fn select_route_module_falls_back_to_path_module_when_targets_are_header_only() {
        let route = targeted_route(
            "/api/guest-example",
            vec![header_target("guest-loop", COHORT_HEADER, "beta")],
        );

        assert_eq!(
            select_route_target_with_roll(&route, &HeaderMap::new(), Some(0))
                .expect("route should fall back to the path module"),
            test_selected_target("guest-example", false)
        );
    }

    #[test]
    fn extract_propagated_headers_copies_legacy_and_canonical_cohort_names() {
        let mut headers = HeaderMap::new();
        headers.insert(COHORT_HEADER, HeaderValue::from_static("beta"));

        assert_eq!(
            extract_propagated_headers(&headers),
            vec![
                PropagatedHeader {
                    name: COHORT_HEADER.to_owned(),
                    value: "beta".to_owned(),
                },
                PropagatedHeader {
                    name: TACHYON_COHORT_HEADER.to_owned(),
                    value: "beta".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn resolve_incoming_hop_limit_defaults_missing_or_invalid_values() {
        let headers = HeaderMap::new();
        assert_eq!(
            resolve_incoming_hop_limit(&headers),
            Ok(HopLimit(DEFAULT_HOP_LIMIT))
        );

        let mut headers = HeaderMap::new();
        headers.insert(HOP_LIMIT_HEADER, HeaderValue::from_static("not-a-number"));
        assert_eq!(
            resolve_incoming_hop_limit(&headers),
            Ok(HopLimit(DEFAULT_HOP_LIMIT))
        );
    }

    #[test]
    fn resolve_incoming_hop_limit_rejects_zero() {
        let mut headers = HeaderMap::new();
        headers.insert(HOP_LIMIT_HEADER, HeaderValue::from_static("0"));

        assert_eq!(resolve_incoming_hop_limit(&headers), Err(()));
    }

    #[test]
    fn resolve_mesh_fetch_target_supports_relative_mesh_routes() {
        let mut config = IntegrityConfig::default_sealed();
        config.host_address = "0.0.0.0:8080".to_owned();
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route(DEFAULT_ROUTE)
            .expect("default route should stay sealed");

        assert_eq!(
            resolve_mesh_fetch_target(&config, &route_registry, caller_route, "/api/guest-loop",)
                .expect("relative mesh route should resolve"),
            "http://127.0.0.1:8080/api/guest-loop"
        );
    }

    #[test]
    fn resolve_mesh_fetch_target_uses_highest_compatible_dependency_version() {
        let mut config = IntegrityConfig::default_sealed();
        config.host_address = "0.0.0.0:8080".to_owned();
        config.routes = vec![
            dependency_route("/api/faas-a", "faas-a", "2.0.0", &[("faas-b", "^2.0")]),
            versioned_route("/api/faas-b-v2", "faas-b", "2.1.0"),
            versioned_route("/api/faas-b-v3", "faas-b", "3.0.0"),
        ];
        let config = validate_integrity_config(config).expect("config should validate");
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route("/api/faas-a")
            .expect("caller route should remain sealed");

        assert_eq!(
            resolve_mesh_fetch_target(
                &config,
                &route_registry,
                caller_route,
                "http://tachyon/faas-b",
            )
            .expect("dependency route should resolve"),
            "http://127.0.0.1:8080/api/faas-b-v2"
        );
    }

    #[test]
    fn resolve_mesh_fetch_target_resolves_internal_resource_aliases() {
        let mut config = IntegrityConfig::default_sealed();
        config.host_address = "0.0.0.0:8080".to_owned();
        config.routes = vec![
            versioned_route("/api/checkout", "checkout", "1.0.0"),
            versioned_route("/api/inventory", "inventory", "1.2.3"),
        ];
        config.resources = BTreeMap::from([(
            "inventory-api".to_owned(),
            IntegrityResource::Internal {
                target: "inventory".to_owned(),
                version_constraint: Some("^1.2".to_owned()),
            },
        )]);
        let config = validate_integrity_config(config).expect("config should validate");
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route("/api/checkout")
            .expect("caller route should remain sealed");

        assert_eq!(
            resolve_mesh_fetch_target(
                &config,
                &route_registry,
                caller_route,
                "http://mesh/inventory-api/items?expand=1",
            )
            .expect("internal resource alias should resolve"),
            "http://127.0.0.1:8080/api/inventory/items?expand=1"
        );
    }

    #[test]
    fn resolve_outbound_http_target_resolves_external_resource_aliases() {
        let mut config = IntegrityConfig::default_sealed();
        config.host_address = "0.0.0.0:8080".to_owned();
        config.routes = vec![versioned_route("/api/checkout", "checkout", "1.0.0")];
        config.resources = BTreeMap::from([(
            "payment-gateway".to_owned(),
            IntegrityResource::External {
                target: "https://api.example.com/v1".to_owned(),
                allowed_methods: vec!["POST".to_owned()],
            },
        )]);
        let config = validate_integrity_config(config).expect("config should validate");
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route("/api/checkout")
            .expect("caller route should remain sealed");

        assert_eq!(
            resolve_outbound_http_target(
                &config,
                &route_registry,
                caller_route,
                &reqwest::Method::POST,
                "http://mesh/payment-gateway/charges?expand=1",
            )
            .expect("external resource alias should resolve"),
            ResolvedOutboundTarget {
                url: "https://api.example.com/v1/charges?expand=1".to_owned(),
                kind: OutboundTargetKind::External,
            }
        );
    }

    #[test]
    fn resolve_outbound_http_target_switches_when_resource_manifest_changes() {
        let routes = vec![
            versioned_route("/api/checkout", "checkout", "1.0.0"),
            versioned_route("/api/service-b", "service-b", "2.1.0"),
        ];
        let caller_path = "/api/checkout";
        let resource_name = "service-b-alias";

        let external_config = validate_integrity_config(IntegrityConfig {
            host_address: "0.0.0.0:8080".to_owned(),
            routes: routes.clone(),
            resources: BTreeMap::from([(
                resource_name.to_owned(),
                IntegrityResource::External {
                    target: "https://api.example.com/v1/service-b".to_owned(),
                    allowed_methods: vec!["GET".to_owned()],
                },
            )]),
            ..IntegrityConfig::default_sealed()
        })
        .expect("external config should validate");
        let external_registry =
            RouteRegistry::build(&external_config).expect("route registry should build");
        let caller_route = external_config
            .sealed_route(caller_path)
            .expect("caller route should remain sealed");
        assert_eq!(
            resolve_outbound_http_target(
                &external_config,
                &external_registry,
                caller_route,
                &reqwest::Method::GET,
                &format!("http://mesh/{resource_name}/health"),
            )
            .expect("external target should resolve")
            .url,
            "https://api.example.com/v1/service-b/health"
        );

        let internal_config = validate_integrity_config(IntegrityConfig {
            host_address: "0.0.0.0:8080".to_owned(),
            routes,
            resources: BTreeMap::from([(
                resource_name.to_owned(),
                IntegrityResource::Internal {
                    target: "service-b".to_owned(),
                    version_constraint: Some("^2.0".to_owned()),
                },
            )]),
            ..IntegrityConfig::default_sealed()
        })
        .expect("internal config should validate");
        let internal_registry =
            RouteRegistry::build(&internal_config).expect("route registry should build");
        let caller_route = internal_config
            .sealed_route(caller_path)
            .expect("caller route should remain sealed");
        assert_eq!(
            resolve_outbound_http_target(
                &internal_config,
                &internal_registry,
                caller_route,
                &reqwest::Method::GET,
                &format!("http://mesh/{resource_name}/health"),
            )
            .expect("internal target should resolve"),
            ResolvedOutboundTarget {
                url: "http://127.0.0.1:8080/api/service-b/health".to_owned(),
                kind: OutboundTargetKind::Internal,
            }
        );
    }

    #[test]
    fn resolve_outbound_http_target_blocks_raw_external_urls_for_user_routes() {
        let config = IntegrityConfig::default_sealed();
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route(DEFAULT_ROUTE)
            .expect("default route should remain sealed");

        let error = resolve_outbound_http_target(
            &config,
            &route_registry,
            caller_route,
            &reqwest::Method::GET,
            "https://api.example.com/v1/ping",
        )
        .expect_err("raw external egress should be rejected for user routes");

        assert!(error.contains("not allowed to call raw external URLs"));
    }

    #[test]
    fn filtered_outbound_http_headers_strips_internal_mesh_headers_for_external_targets() {
        let filtered = filtered_outbound_http_headers(
            vec![
                (HOP_LIMIT_HEADER.to_owned(), "3".to_owned()),
                (
                    TACHYON_IDENTITY_HEADER.to_owned(),
                    "Bearer secret".to_owned(),
                ),
                (
                    "authorization".to_owned(),
                    "Bearer partner-token".to_owned(),
                ),
                ("host".to_owned(), "mesh".to_owned()),
            ],
            &[PropagatedHeader {
                name: COHORT_HEADER.to_owned(),
                value: "beta".to_owned(),
            }],
            &OutboundTargetKind::External,
        );

        assert_eq!(
            filtered,
            vec![(
                "authorization".to_owned(),
                "Bearer partner-token".to_owned(),
            )]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_mesh_response_forwards_propagated_cohort_headers() {
        use axum::{extract::State, routing::get, Router};

        async fn capture_headers(
            State(captured): State<CapturedForwardedHeaders>,
            headers: HeaderMap,
        ) -> &'static str {
            captured
                .lock()
                .expect("captured headers should not be poisoned")
                .push((
                    headers
                        .get(HOP_LIMIT_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned(),
                    headers
                        .get(COHORT_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned(),
                    headers
                        .get(TACHYON_COHORT_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned(),
                    headers
                        .get(TACHYON_IDENTITY_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned(),
                ));
            "ok"
        }

        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/ping", get(capture_headers))
            .with_state(Arc::clone(&captured));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock server should bind");
        let address = listener
            .local_addr()
            .expect("mock server should expose an address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock server should stay healthy");
        });

        let mut inbound_headers = HeaderMap::new();
        inbound_headers.insert(COHORT_HEADER, HeaderValue::from_static("beta"));
        inbound_headers.insert(
            TACHYON_IDENTITY_HEADER,
            HeaderValue::from_static("Bearer spoofed"),
        );
        let propagated_headers = extract_propagated_headers(&inbound_headers);
        let host_identity = test_host_identity(40);
        let mut config = IntegrityConfig::default_sealed();
        config.host_address = address.to_string();
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route(DEFAULT_ROUTE)
            .expect("default route should remain sealed");
        let response = resolve_mesh_response(
            &Client::new(),
            &config,
            &route_registry,
            caller_route,
            host_identity.as_ref(),
            &new_uds_fast_path_registry(),
            HopLimit(DEFAULT_HOP_LIMIT),
            &propagated_headers,
            GuestHttpResponse::new(StatusCode::OK, "MESH_FETCH:/ping"),
        )
        .await
        .expect("mesh fetch should succeed");

        server.abort();

        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, Bytes::from("ok"));
        let captured = captured
            .lock()
            .expect("captured headers should not be poisoned");
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].0, (DEFAULT_HOP_LIMIT - 1).to_string());
        assert_eq!(captured[0].1, "beta");
        assert_eq!(captured[0].2, "beta");
        assert_ne!(captured[0].3, "Bearer spoofed");
        let claims = host_identity
            .verify_token(
                captured[0]
                    .3
                    .strip_prefix("Bearer ")
                    .expect("mesh identity header should include a bearer token"),
            )
            .expect("mesh identity header should verify");
        assert_eq!(claims.route_path, DEFAULT_ROUTE);
        assert_eq!(claims.role, RouteRole::User);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_mesh_response_does_not_leak_identity_headers_to_external_targets() {
        use axum::{extract::State, routing::get, Router};

        async fn capture_identity_header(
            State(captured): State<Arc<std::sync::Mutex<Vec<String>>>>,
            headers: HeaderMap,
        ) -> &'static str {
            captured
                .lock()
                .expect("captured headers should not be poisoned")
                .push(
                    headers
                        .get(TACHYON_IDENTITY_HEADER)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or_default()
                        .to_owned(),
                );
            "ok"
        }

        let captured = Arc::new(std::sync::Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/ping", get(capture_identity_header))
            .with_state(Arc::clone(&captured));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("mock server should bind");
        let address = listener
            .local_addr()
            .expect("mock server should expose an address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock server should stay healthy");
        });

        let config = validate_integrity_config(IntegrityConfig {
            resources: BTreeMap::from([(
                "external-ping".to_owned(),
                IntegrityResource::External {
                    target: format!("http://{address}"),
                    allowed_methods: vec!["GET".to_owned()],
                },
            )]),
            ..IntegrityConfig::default_sealed()
        })
        .expect("config should validate");
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route(DEFAULT_ROUTE)
            .expect("default route should remain sealed");
        let host_identity = test_host_identity(41);
        let response = resolve_mesh_response(
            &Client::new(),
            &config,
            &route_registry,
            caller_route,
            host_identity.as_ref(),
            &new_uds_fast_path_registry(),
            HopLimit(DEFAULT_HOP_LIMIT),
            &[],
            GuestHttpResponse::new(StatusCode::OK, "MESH_FETCH:http://mesh/external-ping/ping"),
        )
        .await
        .expect("external mesh fetch should succeed");

        server.abort();

        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, Bytes::from("ok"));
        assert_eq!(
            captured
                .lock()
                .expect("captured headers should not be poisoned")
                .as_slice(),
            &["".to_owned()]
        );
    }
