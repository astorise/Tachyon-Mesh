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

        let metadata: UdsPeerMetadata = serde_json::from_slice(
            &fs::read(&metadata_files[0]).expect("metadata should be readable"),
        )
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
    async fn resolve_mesh_response_prefers_local_uds_fast_path() {
        use axum::routing::get;

        let discovery_dir = unique_test_dir("tachyon-uds-fast-path");
        let registry = Arc::new(UdsFastPathRegistry::with_discovery_dir(
            discovery_dir.clone(),
        ));
        let mut config = IntegrityConfig::default_sealed();
        config.host_address = "127.0.0.1:19191".to_owned();
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route(DEFAULT_ROUTE)
            .expect("default route should remain sealed");
        let app = axum::Router::new().route("/ping", get(|| async { "uds-fast-path" }));
        let server = start_uds_fast_path_listener(app, &config, Arc::clone(&registry))
            .expect("UDS listener should register")
            .expect("UDS listener should start on Unix");

        tokio::time::sleep(Duration::from_millis(50)).await;

        let response = resolve_mesh_response(
            &Client::new(),
            &config,
            &route_registry,
            caller_route,
            test_host_identity(42).as_ref(),
            registry.as_ref(),
            HopLimit(DEFAULT_HOP_LIMIT),
            &[],
            GuestHttpResponse::new(StatusCode::OK, "MESH_FETCH:/ping"),
        )
        .await
        .expect("UDS fast-path request should succeed");

        server.abort();
        let _ = server.await;
        drop(registry);

        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, Bytes::from("uds-fast-path"));
        let _ = fs::remove_dir_all(discovery_dir);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn resolve_mesh_response_falls_back_to_tcp_when_peer_metadata_is_stale() {
        use axum::routing::get;

        let discovery_dir = unique_test_dir("tachyon-uds-stale-peer");
        let metadata_path = discovery_dir.join("stale-peer.json");
        fs::create_dir_all(&discovery_dir).expect("discovery dir should be created");

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("TCP listener should bind");
        let address = listener
            .local_addr()
            .expect("TCP listener should expose an address");
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route("/ping", get(|| async { "tcp-fallback" })),
            )
            .await
            .expect("TCP fallback server should stay healthy");
        });

        let stale_socket = discovery_dir.join("missing.sock");
        fs::write(
            &metadata_path,
            serde_json::to_vec_pretty(&UdsPeerMetadata {
                host_id: "stale".to_owned(),
                ip: "127.0.0.1".to_owned(),
                socket_path: stale_socket.display().to_string(),
                protocols: vec!["http/1.1".to_owned()],
                pressure_state: PeerPressureState::Idle,
                last_pressure_update_unix_ms: 0,
            })
            .expect("stale metadata should serialize"),
        )
        .expect("stale metadata should be written");

        let registry = UdsFastPathRegistry::with_discovery_dir(discovery_dir.clone());
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
            test_host_identity(43).as_ref(),
            &registry,
            HopLimit(DEFAULT_HOP_LIMIT),
            &[],
            GuestHttpResponse::new(StatusCode::OK, "MESH_FETCH:/ping"),
        )
        .await
        .expect("stale peer should fall back to TCP");

        server.abort();

        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, Bytes::from("tcp-fallback"));
        assert!(
            !metadata_path.exists(),
            "missing-socket metadata should be removed during discovery refresh"
        );
        let _ = fs::remove_dir_all(discovery_dir);
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "manual latency benchmark for UDS fast-path validation"]
    async fn uds_fast_path_is_faster_than_loopback_tcp_for_repeated_mesh_fetches() {
        use axum::routing::get;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("TCP listener should bind");
        let address = listener
            .local_addr()
            .expect("TCP listener should expose an address");
        let tcp_server = tokio::spawn(async move {
            axum::serve(
                listener,
                axum::Router::new().route("/ping", get(|| async { "ok" })),
            )
            .await
            .expect("TCP benchmark server should stay healthy");
        });

        let mut config = IntegrityConfig::default_sealed();
        config.host_address = address.to_string();
        let route_registry = RouteRegistry::build(&config).expect("route registry should build");
        let caller_route = config
            .sealed_route(DEFAULT_ROUTE)
            .expect("default route should remain sealed");
        let host_identity = test_host_identity(44);

        let discovery_dir = unique_test_dir("tachyon-uds-benchmark");
        let uds_registry = Arc::new(UdsFastPathRegistry::with_discovery_dir(
            discovery_dir.clone(),
        ));
        let uds_server = start_uds_fast_path_listener(
            axum::Router::new().route("/ping", get(|| async { "ok" })),
            &config,
            Arc::clone(&uds_registry),
        )
        .expect("UDS benchmark server should register")
        .expect("UDS benchmark server should start");
        tokio::time::sleep(Duration::from_millis(50)).await;

        async fn benchmark(
            registry: &UdsFastPathRegistry,
            config: &IntegrityConfig,
            route_registry: &RouteRegistry,
            caller_route: &IntegrityRoute,
            host_identity: &HostIdentity,
        ) -> Duration {
            let start = Instant::now();
            for _ in 0..24 {
                let response = resolve_mesh_response(
                    &Client::new(),
                    config,
                    route_registry,
                    caller_route,
                    host_identity,
                    registry,
                    HopLimit(DEFAULT_HOP_LIMIT),
                    &[],
                    GuestHttpResponse::new(StatusCode::OK, "MESH_FETCH:/ping"),
                )
                .await
                .expect("benchmark mesh fetch should succeed");
                assert_eq!(response.status, StatusCode::OK);
            }
            start.elapsed()
        }

        let tcp_registry = new_uds_fast_path_registry();
        let tcp_elapsed = benchmark(
            &tcp_registry,
            &config,
            &route_registry,
            caller_route,
            host_identity.as_ref(),
        )
        .await;
        let uds_elapsed = benchmark(
            uds_registry.as_ref(),
            &config,
            &route_registry,
            caller_route,
            host_identity.as_ref(),
        )
        .await;

        uds_server.abort();
        let _ = uds_server.await;
        tcp_server.abort();

        assert!(
            uds_elapsed < tcp_elapsed,
            "UDS fast-path should beat loopback TCP (uds={uds_elapsed:?}, tcp={tcp_elapsed:?})"
        );
        let _ = fs::remove_dir_all(discovery_dir);
    }

    #[tokio::test]
    async fn router_breaks_guest_self_loop_with_http_508() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should expose an address");

        let mut config = IntegrityConfig::default_sealed();
        config.host_address = address.to_string();
        config.routes.push(IntegrityRoute::user("/api/guest-loop"));

        let app = build_app(build_test_state(config, telemetry::init_test_telemetry()));

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("test host should stay healthy");
        });

        let response = Client::new()
            .get(format!("http://{address}/api/guest-loop"))
            .send()
            .await
            .expect("guest-loop request should complete");

        let status = response.status();
        let body = response
            .text()
            .await
            .expect("guest-loop response body should be readable");

        let _ = shutdown_tx.send(());
        server.await.expect("server should shut down cleanly");

        assert_eq!(status, StatusCode::LOOP_DETECTED);
        assert!(
            body.contains("Routing loop detected"),
            "unexpected loop-detected response body: {body}"
        );
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
