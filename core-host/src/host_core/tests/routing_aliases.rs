use super::support_and_cache::*;
use crate::*;

#[cfg(not(feature = "websockets"))]
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
    assert!(
        matches!(&text_frame, Message::Text(text) if *text == "hello"),
        "expected text echo, got: {text_frame:?}"
    );

    client
        .send(Message::Binary(vec![1_u8, 2, 3].into()))
        .await
        .expect("WebSocket client should send binary frame");
    let binary_frame = client
        .next()
        .await
        .expect("WebSocket server should respond to binary frame")
        .expect("WebSocket frame should be valid");
    assert!(
        matches!(&binary_frame, Message::Binary(bytes) if bytes.as_ref() == [1_u8, 2, 3]),
        "expected binary echo, got: {binary_frame:?}"
    );

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
fn injected_signer_route_resolves_to_cert_manager_module() {
    // Zero-touch validation (unknown #1): the sealed signer route
    // `/internal/cert-manager/sign-node` must dispatch to the cert-manager wasm.
    // Module resolution uses targets[0].module, so seal-zero-touch.js sets an
    // explicit target; the route name and path tail are NOT used for the module.
    let route = targeted_route(
        "/internal/cert-manager/sign-node",
        vec![weighted_target("system-faas-cert-manager", 100)],
    );
    let module = select_route_module(&route, &HeaderMap::new())
        .expect("signer route should resolve to a module");
    assert_eq!(module, "system-faas-cert-manager");

    // …and that module name maps to the wasm file the image stages.
    let candidates = guest_module_candidate_paths(&module);
    assert!(
        candidates
            .iter()
            .any(|path| path.ends_with("system_faas_cert_manager.wasm")),
        "expected a system_faas_cert_manager.wasm candidate, got: {candidates:?}"
    );
}

#[test]
fn signer_route_without_target_misresolves() {
    // Guard documenting WHY the explicit target is required: a path-only route
    // resolves to the last segment ("sign-node"), which is NOT cert-manager.
    let route = IntegrityRoute::system("/internal/cert-manager/sign-node");
    let module = select_route_module(&route, &HeaderMap::new()).expect("resolves to a module");
    assert_eq!(module, "sign-node");
    let candidates = guest_module_candidate_paths(&module);
    assert!(
        !candidates
            .iter()
            .any(|path| path.ends_with("system_faas_cert_manager.wasm")),
        "path-only route must NOT resolve to the cert-manager module"
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
async fn local_mesh_dispatch_yields_to_transport_when_route_is_saturated() {
    let mut route = IntegrityRoute::user(DEFAULT_ROUTE);
    route.max_concurrency = 1;
    let config = validate_integrity_config(IntegrityConfig {
        routes: vec![route],
        ..IntegrityConfig::default_sealed()
    })
    .expect("config should validate");
    let state = build_test_state(config, telemetry::init_test_telemetry());
    let runtime = state.runtime.load_full();
    let semaphore = runtime
        .concurrency_limits
        .get(DEFAULT_ROUTE)
        .expect("default route should have a concurrency limiter");
    let _permit = semaphore
        .semaphore
        .clone()
        .try_acquire_owned()
        .expect("test should acquire the only local permit");

    let before =
        mesh_dispatch_total_for(MeshDispatchMode::InProcess, MeshDispatchReason::Saturated);
    let response = try_dispatch_local_mesh_request(
        &state,
        &runtime,
        "http://127.0.0.1:9/api/guest-example",
        &Method::GET,
        &HeaderMap::new(),
        Bytes::new(),
        Vec::new(),
        HopLimit(DEFAULT_HOP_LIMIT),
    )
    .await
    .expect("saturated local route should be a transport fallback");

    assert!(matches!(
        response,
        LocalMeshDispatchAttempt::Fallback(MeshDispatchReason::Saturated)
    ));
    assert_eq!(
        mesh_dispatch_total_for(MeshDispatchMode::InProcess, MeshDispatchReason::Saturated),
        before + 1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn local_mesh_dispatch_overflows_to_peer_when_saturated_and_overflow_allowed() {
    use axum::{body::Bytes as AxumBytes, routing::any, Router};
    use std::sync::Mutex;

    #[derive(Default)]
    struct PeerCapture {
        hop_limits: Vec<String>,
    }

    let peer_capture = Arc::new(Mutex::new(PeerCapture::default()));
    let capture_for_handler = Arc::clone(&peer_capture);
    let peer_app = Router::new().route(
        DEFAULT_ROUTE,
        any(move |headers: HeaderMap, _body: AxumBytes| {
            let capture = Arc::clone(&capture_for_handler);
            async move {
                capture
                    .lock()
                    .expect("peer capture should not be poisoned")
                    .hop_limits
                    .push(
                        headers
                            .get(HOP_LIMIT_HEADER)
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or_default()
                            .to_owned(),
                    );
                (StatusCode::OK, "peer-ok")
            }
        }),
    );
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

    let mut route = IntegrityRoute::user(DEFAULT_ROUTE);
    route.max_concurrency = 1;
    route.allow_overflow = true;
    let config = validate_integrity_config(IntegrityConfig {
        routes: vec![route],
        ..IntegrityConfig::default_sealed()
    })
    .expect("config should validate");
    let state = build_test_state(config, telemetry::init_test_telemetry());
    let runtime = state.runtime.load_full();
    let semaphore = runtime
        .concurrency_limits
        .get(DEFAULT_ROUTE)
        .expect("default route should have a concurrency limiter");
    let _permit = semaphore
        .semaphore
        .clone()
        .try_acquire_owned()
        .expect("test should acquire the only local permit");

    // Simulates `system-faas-mesh-overlay` having already published a
    // healthier peer for this route via `routing_control::update-target`.
    update_control_plane_route_override(
        state.route_overrides.as_ref(),
        &state.peer_capabilities,
        DEFAULT_ROUTE,
        &format!("http://{peer_address}{DEFAULT_ROUTE}"),
    )
    .expect("route override should install");

    let before = mesh_dispatch_total_for(MeshDispatchMode::Peer, MeshDispatchReason::Saturated);
    let response = try_dispatch_local_mesh_request(
        &state,
        &runtime,
        "http://127.0.0.1:9/api/guest-example",
        &Method::GET,
        &HeaderMap::new(),
        Bytes::new(),
        Vec::new(),
        HopLimit(DEFAULT_HOP_LIMIT),
    )
    .await
    .expect("saturated route with an eligible peer should overflow, not error");

    let LocalMeshDispatchAttempt::Handled(handled) = response else {
        panic!("expected the saturated route to be handled via peer overflow");
    };
    assert_eq!(handled.status, StatusCode::OK);
    assert_eq!(&handled.body[..], b"peer-ok");
    assert_eq!(
        mesh_dispatch_total_for(MeshDispatchMode::Peer, MeshDispatchReason::Saturated),
        before + 1
    );

    let captured = peer_capture
        .lock()
        .expect("peer capture should not be poisoned");
    assert_eq!(
        captured.hop_limits,
        vec![HopLimit(DEFAULT_HOP_LIMIT).decremented().to_string()]
    );
    drop(captured);

    peer_server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn local_mesh_dispatch_stays_local_when_no_peer_is_eligible_despite_allow_overflow() {
    let mut route = IntegrityRoute::user(DEFAULT_ROUTE);
    route.max_concurrency = 1;
    route.allow_overflow = true;
    let config = validate_integrity_config(IntegrityConfig {
        routes: vec![route],
        ..IntegrityConfig::default_sealed()
    })
    .expect("config should validate");
    let state = build_test_state(config, telemetry::init_test_telemetry());
    let runtime = state.runtime.load_full();
    let semaphore = runtime
        .concurrency_limits
        .get(DEFAULT_ROUTE)
        .expect("default route should have a concurrency limiter");
    let _permit = semaphore
        .semaphore
        .clone()
        .try_acquire_owned()
        .expect("test should acquire the only local permit");

    // No `route_overrides` entry was ever published for this route, so no
    // peer is eligible: the local queue remains the last resort even though
    // `allow_overflow` is set.
    let before =
        mesh_dispatch_total_for(MeshDispatchMode::InProcess, MeshDispatchReason::Saturated);
    let peer_before =
        mesh_dispatch_total_for(MeshDispatchMode::Peer, MeshDispatchReason::Saturated);
    let response = try_dispatch_local_mesh_request(
        &state,
        &runtime,
        "http://127.0.0.1:9/api/guest-example",
        &Method::GET,
        &HeaderMap::new(),
        Bytes::new(),
        Vec::new(),
        HopLimit(DEFAULT_HOP_LIMIT),
    )
    .await
    .expect("saturated route without an eligible peer should fall back to the local queue");

    assert!(matches!(
        response,
        LocalMeshDispatchAttempt::Fallback(MeshDispatchReason::Saturated)
    ));
    assert_eq!(
        mesh_dispatch_total_for(MeshDispatchMode::InProcess, MeshDispatchReason::Saturated),
        before + 1
    );
    assert_eq!(
        mesh_dispatch_total_for(MeshDispatchMode::Peer, MeshDispatchReason::Saturated),
        peer_before
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn local_mesh_dispatch_honors_bench_force_transport_override() {
    let config = validate_integrity_config(IntegrityConfig {
        routes: vec![IntegrityRoute::user(DEFAULT_ROUTE)],
        ..IntegrityConfig::default_sealed()
    })
    .expect("config should validate");
    let state = build_test_state(config, telemetry::init_test_telemetry());
    let runtime = state.runtime.load_full();

    std::env::set_var(FORCE_MESH_TRANSPORT_ENV, "1");
    let before = mesh_dispatch_total_for(MeshDispatchMode::InProcess, MeshDispatchReason::Remote);
    let response = try_dispatch_local_mesh_request(
        &state,
        &runtime,
        "http://127.0.0.1:9/api/guest-example",
        &Method::GET,
        &HeaderMap::new(),
        Bytes::new(),
        Vec::new(),
        HopLimit(DEFAULT_HOP_LIMIT),
    )
    .await;
    std::env::remove_var(FORCE_MESH_TRANSPORT_ENV);
    let response = response.expect("forced-transport dispatch should not error");

    assert!(matches!(
        response,
        LocalMeshDispatchAttempt::Fallback(MeshDispatchReason::Remote)
    ));
    assert_eq!(
        mesh_dispatch_total_for(MeshDispatchMode::InProcess, MeshDispatchReason::Remote),
        before + 1
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn outbound_http_import_attempts_sealed_local_route_in_process() {
    let mut config = IntegrityConfig::default_sealed();
    config.host_address = "127.0.0.1:9".to_owned();
    let config = validate_integrity_config(config).expect("config should validate");
    let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
    let runtime = state.runtime.load_full();
    let caller_route = config
        .sealed_route(DEFAULT_ROUTE)
        .expect("default route should remain sealed");
    let mut host = ComponentHostState::new(
        caller_route,
        config.clone(),
        runtime.config.guest_memory_limit_bytes,
        state.telemetry.clone(),
        SecretAccess::default(),
        HeaderMap::new(),
        Arc::clone(&state.host_identity),
        Arc::clone(&state.storage_broker),
        Arc::clone(&runtime.concurrency_limits),
        Vec::new(),
        Some(LocalMeshDispatchContext {
            state: state.clone(),
            runtime,
            handle: tokio::runtime::Handle::current(),
        }),
        &[],
    )
    .expect("component host state should build");

    let result = tokio::task::spawn_blocking(move || {
        <ComponentHostState as background_component_bindings::tachyon::mesh::outbound_http::Host>::send_request(
            &mut host,
            "GET".to_owned(),
            "http://mesh/api/guest-example".to_owned(),
            Vec::new(),
            Vec::new(),
        )
    })
    .await
    .expect("outbound HTTP import should run on a blocking worker");

    match result {
        Ok(response) => {
            assert_eq!(response.status, StatusCode::OK.as_u16());
            let body = String::from_utf8_lossy(&response.body);
            assert!(
                body.trim()
                    .starts_with("FaaS received an empty payload | env: missing | secret:"),
                "expected local guest response body, got: {body}"
            );
        }
        Err(error) => {
            assert!(
                error.contains("guest artifact not found"),
                "expected local guest loading failure, got: {error}"
            );
            assert!(
                !error.contains("failed to send") && !error.contains("connection refused"),
                "local sealed outbound HTTP should not fall through to transport: {error}"
            );
        }
    }
}

#[cfg(feature = "websockets")]
#[tokio::test(flavor = "multi_thread")]
async fn websocket_outbound_http_import_attempts_sealed_local_route_in_process() {
    let websocket_route =
        targeted_route("/ws/local", vec![websocket_target("guest-websocket-echo")]);
    let mut config = IntegrityConfig {
        routes: vec![IntegrityRoute::user(DEFAULT_ROUTE), websocket_route],
        ..IntegrityConfig::default_sealed()
    };
    config.host_address = "127.0.0.1:9".to_owned();
    let config = validate_integrity_config(config).expect("config should validate");
    let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
    let runtime = state.runtime.load_full();
    let caller_route = config
        .sealed_route("/ws/local")
        .expect("websocket route should remain sealed");
    let mut host = ComponentHostState::new(
        caller_route,
        config.clone(),
        runtime.config.guest_memory_limit_bytes,
        state.telemetry.clone(),
        SecretAccess::default(),
        HeaderMap::new(),
        Arc::clone(&state.host_identity),
        Arc::clone(&state.storage_broker),
        Arc::clone(&runtime.concurrency_limits),
        Vec::new(),
        Some(LocalMeshDispatchContext {
            state: state.clone(),
            runtime,
            handle: tokio::runtime::Handle::current(),
        }),
        &[],
    )
    .expect("component host state should build");

    let result = tokio::task::spawn_blocking(move || {
        <ComponentHostState as websocket_component_bindings::tachyon::mesh::outbound_http::Host>::send_request(
            &mut host,
            "GET".to_owned(),
            "http://mesh/api/guest-example".to_owned(),
            Vec::new(),
            Vec::new(),
        )
    })
    .await
    .expect("websocket outbound HTTP import should run on a blocking worker");

    match result {
        Ok(response) => {
            assert_eq!(response.status, StatusCode::OK.as_u16());
            let body = String::from_utf8_lossy(&response.body);
            assert!(
                body.trim()
                    .starts_with("FaaS received an empty payload | env: missing | secret:"),
                "expected local guest response body, got: {body}"
            );
        }
        Err(error) => {
            assert!(
                error.contains("guest artifact not found"),
                "expected local guest loading failure, got: {error}"
            );
            assert!(
                !error.contains("failed to send") && !error.contains("connection refused"),
                "websocket sealed outbound HTTP should not fall through to transport: {error}"
            );
        }
    }
}

#[cfg(feature = "websockets")]
#[tokio::test(flavor = "multi_thread")]
async fn websocket_outbound_http_import_yields_to_transport_when_route_is_saturated() {
    let mut local_route = IntegrityRoute::user(DEFAULT_ROUTE);
    local_route.max_concurrency = 1;
    let websocket_route =
        targeted_route("/ws/local", vec![websocket_target("guest-websocket-echo")]);
    let mut config = IntegrityConfig {
        routes: vec![local_route, websocket_route],
        ..IntegrityConfig::default_sealed()
    };
    config.host_address = "127.0.0.1:9".to_owned();
    let config = validate_integrity_config(config).expect("config should validate");
    let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
    let runtime = state.runtime.load_full();
    let semaphore = runtime
        .concurrency_limits
        .get(DEFAULT_ROUTE)
        .expect("default route should have a concurrency limiter");
    let _permit = semaphore
        .semaphore
        .clone()
        .try_acquire_owned()
        .expect("test should acquire the only local permit");
    let caller_route = config
        .sealed_route("/ws/local")
        .expect("websocket route should remain sealed");
    let mut host = ComponentHostState::new(
        caller_route,
        config.clone(),
        runtime.config.guest_memory_limit_bytes,
        state.telemetry.clone(),
        SecretAccess::default(),
        HeaderMap::new(),
        Arc::clone(&state.host_identity),
        Arc::clone(&state.storage_broker),
        Arc::clone(&runtime.concurrency_limits),
        Vec::new(),
        Some(LocalMeshDispatchContext {
            state: state.clone(),
            runtime,
            handle: tokio::runtime::Handle::current(),
        }),
        &[],
    )
    .expect("component host state should build");

    let result = tokio::task::spawn_blocking(move || {
        <ComponentHostState as websocket_component_bindings::tachyon::mesh::outbound_http::Host>::send_request(
            &mut host,
            "GET".to_owned(),
            "http://mesh/api/guest-example".to_owned(),
            Vec::new(),
            Vec::new(),
        )
    })
    .await
    .expect("websocket outbound HTTP import should run on a blocking worker")
    .expect_err("saturated local route should fall back to transport");

    assert!(
        result.contains("failed to send") || result.contains("connection refused"),
        "expected transport fallback failure, got: {result}"
    );
}
