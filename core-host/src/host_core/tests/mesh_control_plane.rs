use super::support_and_cache::*;
use crate::*;

#[tokio::test(flavor = "multi_thread")]
async fn control_plane_gossip_redirects_requests_to_a_healthier_peer() {
    use axum::{
        body::Bytes as AxumBytes,
        extract::State,
        response::IntoResponse,
        routing::{get, post},
        Json, Router,
    };
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Default)]
    struct PeerCapture {
        bodies: Vec<String>,
    }

    async fn gossip_status() -> impl IntoResponse {
        Json(json!({
            "total_requests": 0_u64,
            "completed_requests": 0_u64,
            "error_requests": 0_u64,
            "active_requests": 0_u32,
            "cpu_pressure": 15_u8,
            "ram_pressure": 10_u8,
            "active_instances": 1_u32,
            "allocated_memory_pages": 1_u32,
            "dropped_events": 0_u64,
            "last_status": 200_u16,
            "total_duration_us": 0_u64,
            "total_wasm_duration_us": 0_u64,
            "total_host_overhead_us": 0_u64
        }))
    }

    async fn peer_target(
        State(state): State<Arc<Mutex<PeerCapture>>>,
        body: AxumBytes,
    ) -> impl IntoResponse {
        state
            .lock()
            .expect("peer capture should not be poisoned")
            .bodies
            .push(String::from_utf8_lossy(&body).to_string());
        (StatusCode::OK, "peer-ok")
    }

    let peer_capture = Arc::new(Mutex::new(PeerCapture::default()));
    let peer_app = Router::new()
        .route("/system/gossip", get(gossip_status))
        .route("/api/guest-example", post(peer_target))
        .with_state(Arc::clone(&peer_capture));
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

    let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("host listener should bind");
    let host_address = host_listener
        .local_addr()
        .expect("host listener should expose an address");

    let mut gossip_route = system_targeted_route("/system/gossip", "gossip");
    let peer_urls = format!("http://{peer_address}");
    gossip_route.env = route_env(&[
        ("STEER_ROUTE", "/api/guest-example"),
        ("PEER_URLS", peer_urls.as_str()),
        ("SOFT_LIMIT", "70"),
        ("RECOVER_LIMIT", "50"),
        ("SATURATED_LIMIT", "95"),
        ("BUFFER_ROUTE", "/system/buffer"),
    ]);
    let config = IntegrityConfig {
        host_address: host_address.to_string(),
        routes: vec![
            targeted_route(
                "/api/guest-example",
                vec![weighted_target("guest-example", 100)],
            ),
            gossip_route.clone(),
        ],
        ..IntegrityConfig::default_sealed()
    };
    let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
    state
        .host_load
        .active_instances
        .store(200, Ordering::SeqCst);
    let host_app = build_app(state.clone());
    let host_server = tokio::spawn(async move {
        axum::serve(host_listener, host_app)
            .await
            .expect("host app should stay up");
    });

    tokio::task::spawn_blocking({
        let config = config.clone();
        let route_overrides = Arc::clone(&state.route_overrides);
        let peer_capabilities = Arc::clone(&state.peer_capabilities);
        let host_load = Arc::clone(&state.host_load);
        let storage_broker = Arc::clone(&state.storage_broker);
        let telemetry = state.telemetry.clone();
        let host_identity = Arc::clone(&state.host_identity);
        let host_capabilities = state.host_capabilities;
        move || {
            let engine = build_test_metered_engine(&config);
            let mut runner = BackgroundTickRunner::new(
                &engine,
                &config,
                config
                    .sealed_route("/system/gossip")
                    .expect("gossip route should be sealed"),
                "gossip",
                telemetry,
                build_concurrency_limits(&config),
                host_identity,
                storage_broker,
                route_overrides,
                peer_capabilities,
                host_capabilities,
                host_load,
            )
            .expect("gossip component should instantiate");
            runner.tick().expect("gossip tick should succeed");
        }
    })
    .await
    .expect("gossip task should complete");

    let override_target = state
        .route_overrides
        .load()
        .get("/api/guest-example")
        .cloned()
        .expect("gossip should install a route override");
    assert!(override_target.contains(&format!("http://{peer_address}/api/guest-example")));

    let response = Client::new()
        .post(format!("http://{host_address}/api/guest-example"))
        .body("overflow-request")
        .send()
        .await
        .expect("host request should succeed");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.text().await.expect("response body should decode"),
        "peer-ok"
    );

    let captured = peer_capture
        .lock()
        .expect("peer capture should not be poisoned");
    assert_eq!(captured.bodies, vec!["overflow-request".to_owned()]);

    host_server.abort();
    peer_server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn model_aware_gossip_prefers_peer_with_matching_hot_model() {
    use axum::{
        body::Bytes as AxumBytes,
        extract::State,
        response::IntoResponse,
        routing::{get, post},
        Json, Router,
    };
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Default)]
    struct PeerCapture {
        bodies: Vec<String>,
    }

    async fn wrong_model_gossip() -> impl IntoResponse {
        Json(json!({
            "total_requests": 0_u64,
            "completed_requests": 0_u64,
            "error_requests": 0_u64,
            "active_requests": 1_u32,
            "cpu_pressure": 10_u8,
            "ram_pressure": 10_u8,
            "active_instances": 1_u32,
            "allocated_memory_pages": 1_u32,
            "hot_models": ["mistral"],
            "dropped_events": 0_u64,
            "last_status": 200_u16,
            "total_duration_us": 0_u64,
            "total_wasm_duration_us": 0_u64,
            "total_host_overhead_us": 0_u64
        }))
    }

    async fn right_model_gossip() -> impl IntoResponse {
        Json(json!({
            "total_requests": 0_u64,
            "completed_requests": 0_u64,
            "error_requests": 0_u64,
            "active_requests": 2_u32,
            "cpu_pressure": 20_u8,
            "ram_pressure": 20_u8,
            "active_instances": 1_u32,
            "allocated_memory_pages": 1_u32,
            "hot_models": ["llama3"],
            "dropped_events": 0_u64,
            "last_status": 200_u16,
            "total_duration_us": 0_u64,
            "total_wasm_duration_us": 0_u64,
            "total_host_overhead_us": 0_u64
        }))
    }

    async fn peer_target(
        State(state): State<Arc<Mutex<PeerCapture>>>,
        body: AxumBytes,
    ) -> impl IntoResponse {
        state
            .lock()
            .expect("peer capture should not be poisoned")
            .bodies
            .push(String::from_utf8_lossy(&body).to_string());
        (StatusCode::OK, "peer-match")
    }

    let wrong_capture = Arc::new(Mutex::new(PeerCapture::default()));
    let wrong_app = Router::new()
        .route("/system/gossip", get(wrong_model_gossip))
        .route("/api/guest-ai", post(peer_target))
        .with_state(Arc::clone(&wrong_capture));
    let wrong_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("wrong-model peer should bind");
    let wrong_address = wrong_listener
        .local_addr()
        .expect("wrong-model peer should expose an address");
    let wrong_server = tokio::spawn(async move {
        axum::serve(wrong_listener, wrong_app)
            .await
            .expect("wrong-model peer should stay up");
    });

    let right_capture = Arc::new(Mutex::new(PeerCapture::default()));
    let right_app = Router::new()
        .route("/system/gossip", get(right_model_gossip))
        .route("/api/guest-ai", post(peer_target))
        .with_state(Arc::clone(&right_capture));
    let right_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("matching-model peer should bind");
    let right_address = right_listener
        .local_addr()
        .expect("matching-model peer should expose an address");
    let right_server = tokio::spawn(async move {
        axum::serve(right_listener, right_app)
            .await
            .expect("matching-model peer should stay up");
    });

    let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("host listener should bind");
    let host_address = host_listener
        .local_addr()
        .expect("host listener should expose an address");

    let mut user_route =
        targeted_route("/api/guest-ai", vec![weighted_target("guest-example", 100)]);
    user_route.models = vec![IntegrityModelBinding {
        alias: "llama3".to_owned(),
        path: "/models/llama3.gguf".to_owned(),
        device: ModelDevice::Cuda,
        qos: RouteQos::RealTime,
    }];
    let mut gossip_route = system_targeted_route("/system/gossip", "gossip");
    let peer_urls = format!("http://{wrong_address},http://{right_address}");
    gossip_route.env = route_env(&[
        ("STEER_ROUTE", "/api/guest-ai"),
        ("PEER_URLS", peer_urls.as_str()),
        ("SOFT_LIMIT", "70"),
        ("RECOVER_LIMIT", "50"),
        ("SATURATED_LIMIT", "95"),
        ("BUFFER_ROUTE", "/system/buffer"),
    ]);
    let config = IntegrityConfig {
        host_address: host_address.to_string(),
        routes: vec![user_route, gossip_route.clone()],
        ..IntegrityConfig::default_sealed()
    };
    let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
    state
        .host_load
        .active_instances
        .store(200, Ordering::SeqCst);
    let host_app = build_app(state.clone());
    let host_server = tokio::spawn(async move {
        axum::serve(host_listener, host_app)
            .await
            .expect("host app should stay up");
    });

    tokio::task::spawn_blocking({
        let config = config.clone();
        let route_overrides = Arc::clone(&state.route_overrides);
        let peer_capabilities = Arc::clone(&state.peer_capabilities);
        let host_load = Arc::clone(&state.host_load);
        let storage_broker = Arc::clone(&state.storage_broker);
        let telemetry = state.telemetry.clone();
        let host_identity = Arc::clone(&state.host_identity);
        let host_capabilities = state.host_capabilities;
        move || {
            let engine = build_test_metered_engine(&config);
            let mut runner = BackgroundTickRunner::new(
                &engine,
                &config,
                config
                    .sealed_route("/system/gossip")
                    .expect("gossip route should be sealed"),
                "gossip",
                telemetry,
                build_concurrency_limits(&config),
                host_identity,
                storage_broker,
                route_overrides,
                peer_capabilities,
                host_capabilities,
                host_load,
            )
            .expect("gossip component should instantiate");
            runner.tick().expect("gossip tick should succeed");
        }
    })
    .await
    .expect("gossip task should complete");

    let override_target = state
        .route_overrides
        .load()
        .get("/api/guest-ai")
        .cloned()
        .expect("gossip should install a route override");
    let descriptor: RouteOverrideDescriptor =
        serde_json::from_str(&override_target).expect("override descriptor should parse");
    assert_eq!(descriptor.candidates.len(), 2);

    let response = Client::new()
        .post(format!("http://{host_address}/api/guest-ai"))
        .header("x-tachyon-model", "llama3")
        .body("hot-model-request")
        .send()
        .await
        .expect("host request should succeed");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.text().await.expect("response body should decode"),
        "peer-match"
    );

    let wrong_capture = wrong_capture
        .lock()
        .expect("wrong-model capture should not be poisoned");
    assert!(wrong_capture.bodies.is_empty());
    let right_capture = right_capture
        .lock()
        .expect("matching-model capture should not be poisoned");
    assert_eq!(right_capture.bodies, vec!["hot-model-request".to_owned()]);

    host_server.abort();
    wrong_server.abort();
    right_server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn model_aware_gossip_keeps_request_local_when_no_peer_has_hot_model() {
    use axum::{
        body::Bytes as AxumBytes,
        extract::State,
        response::IntoResponse,
        routing::{get, post},
        Json, Router,
    };
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Default)]
    struct PeerCapture {
        bodies: Vec<String>,
    }

    async fn cold_peer_gossip() -> impl IntoResponse {
        Json(json!({
            "total_requests": 0_u64,
            "completed_requests": 0_u64,
            "error_requests": 0_u64,
            "active_requests": 0_u32,
            "cpu_pressure": 10_u8,
            "ram_pressure": 10_u8,
            "active_instances": 1_u32,
            "allocated_memory_pages": 1_u32,
            "hot_models": ["mistral"],
            "dropped_events": 0_u64,
            "last_status": 200_u16,
            "total_duration_us": 0_u64,
            "total_wasm_duration_us": 0_u64,
            "total_host_overhead_us": 0_u64
        }))
    }

    async fn peer_target(
        State(state): State<Arc<Mutex<PeerCapture>>>,
        body: AxumBytes,
    ) -> impl IntoResponse {
        state
            .lock()
            .expect("peer capture should not be poisoned")
            .bodies
            .push(String::from_utf8_lossy(&body).to_string());
        (StatusCode::OK, "peer-cold")
    }

    let peer_capture = Arc::new(Mutex::new(PeerCapture::default()));
    let peer_app = Router::new()
        .route("/system/gossip", get(cold_peer_gossip))
        .route("/api/guest-ai", post(peer_target))
        .with_state(Arc::clone(&peer_capture));
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

    let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("host listener should bind");
    let host_address = host_listener
        .local_addr()
        .expect("host listener should expose an address");

    let mut user_route =
        targeted_route("/api/guest-ai", vec![weighted_target("guest-example", 100)]);
    user_route.models = vec![IntegrityModelBinding {
        alias: "llama3".to_owned(),
        path: "/models/llama3.gguf".to_owned(),
        device: ModelDevice::Cuda,
        qos: RouteQos::RealTime,
    }];
    let mut gossip_route = system_targeted_route("/system/gossip", "gossip");
    let peer_urls = format!("http://{peer_address}");
    gossip_route.env = route_env(&[
        ("STEER_ROUTE", "/api/guest-ai"),
        ("PEER_URLS", peer_urls.as_str()),
        ("SOFT_LIMIT", "70"),
        ("RECOVER_LIMIT", "50"),
        ("SATURATED_LIMIT", "95"),
        ("BUFFER_ROUTE", "/system/buffer"),
    ]);
    let config = IntegrityConfig {
        host_address: host_address.to_string(),
        routes: vec![user_route, gossip_route.clone()],
        ..IntegrityConfig::default_sealed()
    };
    let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
    state
        .host_load
        .active_instances
        .store(200, Ordering::SeqCst);
    let host_app = build_app(state.clone());
    let host_server = tokio::spawn(async move {
        axum::serve(host_listener, host_app)
            .await
            .expect("host app should stay up");
    });

    tokio::task::spawn_blocking({
        let config = config.clone();
        let route_overrides = Arc::clone(&state.route_overrides);
        let peer_capabilities = Arc::clone(&state.peer_capabilities);
        let host_load = Arc::clone(&state.host_load);
        let storage_broker = Arc::clone(&state.storage_broker);
        let telemetry = state.telemetry.clone();
        let host_identity = Arc::clone(&state.host_identity);
        let host_capabilities = state.host_capabilities;
        move || {
            let engine = build_test_metered_engine(&config);
            let mut runner = BackgroundTickRunner::new(
                &engine,
                &config,
                config
                    .sealed_route("/system/gossip")
                    .expect("gossip route should be sealed"),
                "gossip",
                telemetry,
                build_concurrency_limits(&config),
                host_identity,
                storage_broker,
                route_overrides,
                peer_capabilities,
                host_capabilities,
                host_load,
            )
            .expect("gossip component should instantiate");
            runner.tick().expect("gossip tick should succeed");
        }
    })
    .await
    .expect("gossip task should complete");

    let response = Client::new()
        .post(format!("http://{host_address}/api/guest-ai"))
        .header("x-tachyon-model", "llama3")
        .body("local-only")
        .send()
        .await
        .expect("host request should succeed");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.text().await.expect("response body should decode"),
        expected_guest_example_body_without_secret_grant("FaaS received: local-only")
    );

    let peer_capture = peer_capture
        .lock()
        .expect("peer capture should not be poisoned");
    assert!(peer_capture.bodies.is_empty());

    host_server.abort();
    peer_server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn capability_routing_skips_override_candidates_without_required_capabilities() {
    use axum::{
        body::Bytes as AxumBytes, extract::State, response::IntoResponse, routing::post, Router,
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct PeerCapture {
        bodies: Vec<String>,
    }

    async fn peer_target(
        State(state): State<Arc<Mutex<PeerCapture>>>,
        body: AxumBytes,
    ) -> impl IntoResponse {
        state
            .lock()
            .expect("peer capture should not be poisoned")
            .bodies
            .push(String::from_utf8_lossy(&body).to_string());
        (StatusCode::OK, "peer-capable")
    }

    let wrong_capture = Arc::new(Mutex::new(PeerCapture::default()));
    let wrong_app = Router::new()
        .route("/api/guest-example", post(peer_target))
        .with_state(Arc::clone(&wrong_capture));
    let wrong_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("incapable peer should bind");
    let wrong_address = wrong_listener
        .local_addr()
        .expect("incapable peer should expose an address");
    let wrong_server = tokio::spawn(async move {
        axum::serve(wrong_listener, wrong_app)
            .await
            .expect("incapable peer should stay up");
    });

    let right_capture = Arc::new(Mutex::new(PeerCapture::default()));
    let right_app = Router::new()
        .route("/api/guest-example", post(peer_target))
        .with_state(Arc::clone(&right_capture));
    let right_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("capable peer should bind");
    let right_address = right_listener
        .local_addr()
        .expect("capable peer should expose an address");
    let right_server = tokio::spawn(async move {
        axum::serve(right_listener, right_app)
            .await
            .expect("capable peer should stay up");
    });

    let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("host listener should bind");
    let host_address = host_listener
        .local_addr()
        .expect("host listener should expose an address");
    let config = IntegrityConfig {
        host_address: host_address.to_string(),
        routes: vec![targeted_route(
            "/api/guest-example",
            vec![capability_target(
                "guest-example",
                &["core:wasi", "accel:cuda"],
            )],
        )],
        ..IntegrityConfig::default_sealed()
    };
    let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
    let host_app = build_app(state.clone());
    let host_server = tokio::spawn(async move {
        axum::serve(host_listener, host_app)
            .await
            .expect("host app should stay up");
    });

    let override_descriptor = serde_json::to_string(&RouteOverrideDescriptor {
        candidates: vec![
            RouteOverrideCandidate {
                destination: format!("http://{wrong_address}/api/guest-example"),
                hot_models: Vec::new(),
                effective_pressure: 5,
                capability_mask: Capabilities::CORE_WASI,
                capabilities: vec!["core:wasi".to_owned()],
            },
            RouteOverrideCandidate {
                destination: format!("http://{right_address}/api/guest-example"),
                hot_models: Vec::new(),
                effective_pressure: 10,
                capability_mask: Capabilities::CORE_WASI | Capabilities::ACCEL_CUDA,
                capabilities: vec!["core:wasi".to_owned(), "accel:cuda".to_owned()],
            },
        ],
    })
    .expect("override descriptor should serialize");
    update_control_plane_route_override(
        state.route_overrides.as_ref(),
        &state.peer_capabilities,
        "/api/guest-example",
        &override_descriptor,
    )
    .expect("capability-aware override should install");

    let response = Client::new()
        .post(format!("http://{host_address}/api/guest-example"))
        .body("capability-request")
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.text().await.expect("response body should decode"),
        "peer-capable"
    );

    let wrong_capture = wrong_capture
        .lock()
        .expect("incapable peer capture should not be poisoned");
    assert!(wrong_capture.bodies.is_empty());
    let right_capture = right_capture
        .lock()
        .expect("capable peer capture should not be poisoned");
    assert_eq!(right_capture.bodies, vec!["capability-request".to_owned()]);
    let cached = state
        .peer_capabilities
        .lock()
        .expect("peer cache should not be poisoned");
    assert_eq!(cached.len(), 2);

    host_server.abort();
    wrong_server.abort();
    right_server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn capability_routing_returns_503_when_local_and_mesh_lack_requirements() {
    let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("host listener should bind");
    let host_address = host_listener
        .local_addr()
        .expect("host listener should expose an address");
    let config = IntegrityConfig {
        host_address: host_address.to_string(),
        routes: vec![targeted_route(
            "/api/guest-example",
            vec![capability_target(
                "guest-example",
                &["core:wasi", "os:linux", "os:windows"],
            )],
        )],
        ..IntegrityConfig::default_sealed()
    };
    let state = build_test_state(config, telemetry::init_test_telemetry());
    let host_app = build_app(state);
    let host_server = tokio::spawn(async move {
        axum::serve(host_listener, host_app)
            .await
            .expect("host app should stay up");
    });

    let response = Client::new()
        .post(format!("http://{host_address}/api/guest-example"))
        .body("missing-capability")
        .send()
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = response.text().await.expect("response body should decode");
    assert!(body.contains("Missing Capability"));
    assert!(body.contains("os:linux") || body.contains("os:windows"));

    host_server.abort();
}

#[cfg(feature = "ai-inference")]
#[tokio::test(flavor = "multi_thread")]
async fn mesh_qos_router_forwards_realtime_gpu_requests_to_prefixed_override() {
    use axum::{
        body::Bytes as AxumBytes, extract::State, response::IntoResponse, routing::post, Router,
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct PeerCapture {
        bodies: Vec<String>,
    }

    async fn peer_target(
        State(state): State<Arc<Mutex<PeerCapture>>>,
        body: AxumBytes,
    ) -> impl IntoResponse {
        state
            .lock()
            .expect("peer capture should not be poisoned")
            .bodies
            .push(String::from_utf8_lossy(&body).to_string());
        (StatusCode::OK, "peer-realtime")
    }

    let peer_capture = Arc::new(Mutex::new(PeerCapture::default()));
    let peer_app = Router::new()
        .route("/api/guest-ai", post(peer_target))
        .with_state(Arc::clone(&peer_capture));
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

    let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("host listener should bind");
    let host_address = host_listener
        .local_addr()
        .expect("host listener should expose an address");

    let mut route = targeted_route("/api/guest-ai", vec![weighted_target("guest-example", 100)]);
    route.models = vec![
        IntegrityModelBinding {
            alias: "gpu-live-chat".to_owned(),
            path: "/models/gpu-live-chat.gguf".to_owned(),
            device: ModelDevice::Cuda,
            qos: RouteQos::RealTime,
        },
        IntegrityModelBinding {
            alias: "gpu-batch".to_owned(),
            path: "/models/gpu-batch.gguf".to_owned(),
            device: ModelDevice::Cuda,
            qos: RouteQos::Batch,
        },
    ];
    let config = IntegrityConfig {
        host_address: host_address.to_string(),
        routes: vec![route],
        ..IntegrityConfig::default_sealed()
    };
    let state = build_test_state(config, telemetry::init_test_telemetry());
    state.runtime.load().ai_runtime.set_queue_depth_for_test(
        ai_inference::AcceleratorKind::Gpu,
        RouteQos::RealTime,
        2,
    );
    update_control_plane_route_override(
        state.route_overrides.as_ref(),
        &state.peer_capabilities,
        &format!("{MESH_QOS_OVERRIDE_PREFIX}/api/guest-ai"),
        &format!("http://{peer_address}/api/guest-ai"),
    )
    .expect("mesh qos override should install");

    let host_app = build_app(state);
    let host_server = tokio::spawn(async move {
        axum::serve(host_listener, host_app)
            .await
            .expect("host app should stay up");
    });

    let response = Client::new()
        .post(format!("http://{host_address}/api/guest-ai"))
        .header("x-tachyon-model", "gpu-live-chat")
        .body("realtime-request")
        .send()
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.text().await.expect("response body should decode"),
        "peer-realtime"
    );
    let peer_capture = peer_capture
        .lock()
        .expect("peer capture should not be poisoned");
    assert_eq!(peer_capture.bodies, vec!["realtime-request".to_owned()]);

    host_server.abort();
    peer_server.abort();
}

#[cfg(feature = "ai-inference")]
#[tokio::test(flavor = "multi_thread")]
async fn mesh_qos_router_keeps_batch_gpu_requests_local_below_remote_threshold() {
    use axum::{
        body::Bytes as AxumBytes, extract::State, response::IntoResponse, routing::post, Router,
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct PeerCapture {
        bodies: Vec<String>,
    }

    async fn peer_target(
        State(state): State<Arc<Mutex<PeerCapture>>>,
        body: AxumBytes,
    ) -> impl IntoResponse {
        state
            .lock()
            .expect("peer capture should not be poisoned")
            .bodies
            .push(String::from_utf8_lossy(&body).to_string());
        (StatusCode::OK, "peer-batch")
    }

    let peer_capture = Arc::new(Mutex::new(PeerCapture::default()));
    let peer_app = Router::new()
        .route("/api/guest-ai", post(peer_target))
        .with_state(Arc::clone(&peer_capture));
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

    let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("host listener should bind");
    let host_address = host_listener
        .local_addr()
        .expect("host listener should expose an address");

    let mut route = targeted_route("/api/guest-ai", vec![weighted_target("guest-example", 100)]);
    route.models = vec![
        IntegrityModelBinding {
            alias: "gpu-live-chat".to_owned(),
            path: "/models/gpu-live-chat.gguf".to_owned(),
            device: ModelDevice::Cuda,
            qos: RouteQos::RealTime,
        },
        IntegrityModelBinding {
            alias: "gpu-batch".to_owned(),
            path: "/models/gpu-batch.gguf".to_owned(),
            device: ModelDevice::Cuda,
            qos: RouteQos::Batch,
        },
    ];
    let config = IntegrityConfig {
        host_address: host_address.to_string(),
        routes: vec![route],
        ..IntegrityConfig::default_sealed()
    };
    let state = build_test_state(config, telemetry::init_test_telemetry());
    state.runtime.load().ai_runtime.set_queue_depth_for_test(
        ai_inference::AcceleratorKind::Gpu,
        RouteQos::Batch,
        32,
    );
    update_control_plane_route_override(
        state.route_overrides.as_ref(),
        &state.peer_capabilities,
        &format!("{MESH_QOS_OVERRIDE_PREFIX}/api/guest-ai"),
        &format!("http://{peer_address}/api/guest-ai"),
    )
    .expect("mesh qos override should install");

    let host_app = build_app(state);
    let host_server = tokio::spawn(async move {
        axum::serve(host_listener, host_app)
            .await
            .expect("host app should stay up");
    });

    let response = Client::new()
        .post(format!("http://{host_address}/api/guest-ai"))
        .header("x-tachyon-model", "gpu-batch")
        .body("batch-request")
        .send()
        .await
        .expect("request should complete");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.text().await.expect("response body should decode"),
        expected_guest_example_body_without_secret_grant("FaaS received: batch-request")
    );
    let peer_capture = peer_capture
        .lock()
        .expect("peer capture should not be poisoned");
    assert!(peer_capture.bodies.is_empty());

    host_server.abort();
    peer_server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn control_plane_buffer_persists_and_replays_requests_when_capacity_returns() {
    let queue_dir = unique_test_dir("tachyon-buffer-queue");
    let state_dir = unique_test_dir("tachyon-buffer-state");
    fs::create_dir_all(&queue_dir).expect("buffer queue dir should create");
    fs::create_dir_all(&state_dir).expect("buffer state dir should create");

    let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("host listener should bind");
    let host_address = host_listener
        .local_addr()
        .expect("host listener should expose an address");

    let mut buffer_route = system_targeted_route("/system/buffer", "buffer");
    buffer_route.env = route_env(&[
        ("BUFFER_DIR", "/buffer"),
        ("RAM_QUEUE_CAPACITY", "1"),
        ("REPLAY_CPU_LIMIT", "70"),
        ("REPLAY_RAM_LIMIT", "70"),
        ("REPLAY_BATCH_SIZE", "4"),
    ]);
    buffer_route.volumes = vec![mounted_volume(&queue_dir, "/buffer")];

    let mut user_route = targeted_route(
        "/api/guest-volume",
        vec![weighted_target("guest-volume", 100)],
    );
    user_route.volumes = vec![mounted_volume(&state_dir, "/app/data")];
    let config = IntegrityConfig {
        host_address: host_address.to_string(),
        routes: vec![user_route.clone(), buffer_route.clone()],
        ..IntegrityConfig::default_sealed()
    };
    let state = build_test_state(config.clone(), telemetry::init_test_telemetry());
    update_control_plane_route_override(
        state.route_overrides.as_ref(),
        &state.peer_capabilities,
        "/api/guest-volume",
        "/system/buffer",
    )
    .expect("buffer override should install");

    let host_app = build_app(state.clone());
    let host_server = tokio::spawn(async move {
        axum::serve(host_listener, host_app)
            .await
            .expect("host app should stay up");
    });

    let response = Client::new()
        .post(format!("http://{host_address}/api/guest-volume"))
        .body("buffered payload")
        .send()
        .await
        .expect("buffered request should succeed");
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let queued_files = fs::read_dir(queue_dir.join("ram"))
        .expect("ram queue should exist")
        .filter_map(Result::ok)
        .count();
    assert_eq!(queued_files, 1);

    tokio::task::spawn_blocking({
        let config = config.clone();
        let route_overrides = Arc::clone(&state.route_overrides);
        let peer_capabilities = Arc::clone(&state.peer_capabilities);
        let host_load = Arc::clone(&state.host_load);
        let storage_broker = Arc::clone(&state.storage_broker);
        let telemetry = state.telemetry.clone();
        let host_identity = Arc::clone(&state.host_identity);
        let host_capabilities = state.host_capabilities;
        move || {
            let engine = build_test_metered_engine(&config);
            let mut runner = BackgroundTickRunner::new(
                &engine,
                &config,
                config
                    .sealed_route("/system/buffer")
                    .expect("buffer route should be sealed"),
                "buffer",
                telemetry,
                build_concurrency_limits(&config),
                host_identity,
                storage_broker,
                route_overrides,
                peer_capabilities,
                host_capabilities,
                host_load,
            )
            .expect("buffer component should instantiate");
            runner.tick().expect("buffer tick should succeed");
        }
    })
    .await
    .expect("buffer replay task should complete");

    let persisted = fs::read_to_string(state_dir.join("state.txt"))
        .expect("guest-volume should persist replayed payload");
    assert_eq!(persisted, "buffered payload");

    let remaining = fs::read_dir(queue_dir.join("ram"))
        .expect("ram queue should still exist")
        .filter_map(Result::ok)
        .count();
    assert_eq!(remaining, 0);

    host_server.abort();
}

#[test]
fn blocking_reqwest_client_initializes_with_default_tls_provider() {
    ensure_rustls_crypto_provider();
    let _client = blocking_outbound_http_client();
}
