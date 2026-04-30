use super::support_and_cache::*;
use crate::*;

#[tokio::test(flavor = "multi_thread")]
async fn bridge_manager_relays_packets_between_allocated_ports() {
    let manager = BridgeManager::default();
    let client_a = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("client A should bind");
    let client_b = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("client B should bind");

    let allocation = manager
        .create_relay(BridgeConfig {
            client_a_addr: client_a
                .local_addr()
                .expect("client A address should resolve")
                .to_string(),
            client_b_addr: client_b
                .local_addr()
                .expect("client B address should resolve")
                .to_string(),
            timeout_seconds: 5,
        })
        .expect("bridge allocation should succeed");
    assert_eq!(manager.active_relay_count(), 1);

    client_a
        .send_to(
            b"alpha",
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), allocation.port_a),
        )
        .await
        .expect("client A should send through the bridge");
    let mut received = [0_u8; 16];
    let (size, source) =
        tokio::time::timeout(Duration::from_secs(1), client_b.recv_from(&mut received))
            .await
            .expect("bridge delivery to client B should not time out")
            .expect("client B should receive relayed datagram");
    assert_eq!(&received[..size], b"alpha");
    assert_eq!(source.port(), allocation.port_b);

    client_b
        .send_to(
            b"beta",
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), allocation.port_b),
        )
        .await
        .expect("client B should send through the bridge");
    let (size, source) =
        tokio::time::timeout(Duration::from_secs(1), client_a.recv_from(&mut received))
            .await
            .expect("bridge delivery to client A should not time out")
            .expect("client A should receive relayed datagram");
    assert_eq!(&received[..size], b"beta");
    assert_eq!(source.port(), allocation.port_a);
    assert!(manager.total_relayed_bytes() >= 9);

    manager
        .destroy_relay(&allocation.bridge_id)
        .expect("bridge teardown should succeed");
    assert_eq!(manager.active_relay_count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn voip_gate_allocates_bridge_via_system_route_and_relays_packets() {
    let session_dir = unique_test_dir("tachyon-bridge-sessions");
    fs::create_dir_all(&session_dir).expect("session dir should exist");
    let client_a = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("client A should bind");
    let client_b = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("client B should bind");
    let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("host listener should bind");
    let host_address = host_listener
        .local_addr()
        .expect("host listener should expose a local address");

    let mut bridge_route = system_targeted_route(SYSTEM_BRIDGE_ROUTE, "system-faas-bridge");
    bridge_route.volumes = vec![mounted_ram_volume(&session_dir, "/sessions")];
    let config = validate_integrity_config(IntegrityConfig {
        host_address: host_address.to_string(),
        routes: vec![
            targeted_route(
                "/api/voip-gate",
                vec![weighted_target("guest-voip-gate", 100)],
            ),
            bridge_route,
        ],
        ..IntegrityConfig::default_sealed()
    })
    .expect("bridge config should validate");
    let state = build_test_state(config, telemetry::init_test_telemetry());
    let app = build_app(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(host_listener, app)
            .await
            .expect("bridge test app should stay up");
    });

    let allocation = Client::new()
            .post(format!("http://{host_address}/api/voip-gate"))
            .header("content-type", "application/json")
            .body(
                serde_json::to_vec(&serde_json::json!({
                    "client_a_addr": client_a.local_addr().expect("client A address should resolve").to_string(),
                    "client_b_addr": client_b.local_addr().expect("client B address should resolve").to_string(),
                    "timeout_seconds": 5
                }))
                .expect("voip gate request body should serialize"),
            )
            .send()
            .await
            .expect("voip gate request should succeed")
            .error_for_status()
            .expect("voip gate response should be OK")
            .bytes()
            .await
            .map(|body| {
                serde_json::from_slice::<BridgeAllocation>(&body)
                    .expect("bridge allocation response should decode")
            })
            .expect("voip gate response body should read");
    assert_eq!(allocation.ip, Ipv4Addr::LOCALHOST.to_string());

    client_a
        .send_to(
            b"hello bridge",
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), allocation.port_a),
        )
        .await
        .expect("client A should send a bridged datagram");
    let mut buffer = [0_u8; 32];
    let (size, source) =
        tokio::time::timeout(Duration::from_secs(1), client_b.recv_from(&mut buffer))
            .await
            .expect("client B delivery should not time out")
            .expect("client B should receive bridged datagram");
    assert_eq!(&buffer[..size], b"hello bridge");
    assert_eq!(source.port(), allocation.port_b);

    let persisted = fs::read_to_string(session_dir.join(format!("{}.json", allocation.bridge_id)))
        .expect("system bridge should persist the active session");
    assert!(persisted.contains("\"status\":\"active\""));
    assert_eq!(state.bridge_manager.active_relay_count(), 1);
    state
        .bridge_manager
        .destroy_relay(&allocation.bridge_id)
        .expect("bridge teardown should succeed");
    assert_eq!(state.bridge_manager.active_relay_count(), 0);

    server.abort();
    let _ = server.await;
    let _ = fs::remove_dir_all(session_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn voip_gate_delegates_bridge_to_healthier_peer_when_local_l4_is_saturated() {
    use axum::{
        body::Bytes as AxumBytes,
        extract::State,
        response::IntoResponse,
        routing::{get, post},
        Json, Router,
    };
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct PeerCapture {
        headers: Vec<Vec<(String, String)>>,
        bodies: Vec<String>,
    }

    async fn peer_gossip() -> impl IntoResponse {
        Json(json!({
            "total_requests": 0_u64,
            "completed_requests": 0_u64,
            "error_requests": 0_u64,
            "active_requests": 1_u32,
            "cpu_pressure": 5_u8,
            "ram_pressure": 5_u8,
            "active_instances": 1_u32,
            "allocated_memory_pages": 1_u32,
            "capability_mask": 0_u64,
            "capabilities": [],
            "active_l4_relays": 1_u32,
            "l4_throughput_bytes_per_sec": 1024_u64,
            "l4_load_score": 10_u8,
            "advertise_ip": "203.0.113.50",
            "cpu_rt_load": 0_u32,
            "cpu_standard_load": 0_u32,
            "cpu_batch_load": 0_u32,
            "gpu_rt_load": 0_u32,
            "gpu_standard_load": 0_u32,
            "gpu_batch_load": 0_u32,
            "npu_rt_load": 0_u32,
            "npu_standard_load": 0_u32,
            "npu_batch_load": 0_u32,
            "tpu_rt_load": 0_u32,
            "tpu_standard_load": 0_u32,
            "tpu_batch_load": 0_u32,
            "hot_models": [],
            "dropped_events": 0_u64,
            "last_status": 200_u16,
            "total_duration_us": 0_u64,
            "total_wasm_duration_us": 0_u64,
            "total_host_overhead_us": 0_u64
        }))
    }

    async fn peer_bridge(
        State(state): State<Arc<Mutex<PeerCapture>>>,
        headers: HeaderMap,
        body: AxumBytes,
    ) -> impl IntoResponse {
        let mut capture = state.lock().expect("peer capture should not be poisoned");
        capture.headers.push(
            headers
                .iter()
                .map(|(name, value)| {
                    (
                        name.as_str().to_owned(),
                        value.to_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect(),
        );
        capture
            .bodies
            .push(String::from_utf8_lossy(&body).to_string());
        (
            StatusCode::OK,
            Json(json!({
                "bridge_id": "peer-bridge-1",
                "ip": "203.0.113.50",
                "port_a": 31_000_u16,
                "port_b": 31_001_u16
            })),
        )
    }

    let peer_capture = Arc::new(Mutex::new(PeerCapture::default()));
    let peer_app = Router::new()
        .route("/system/gossip", get(peer_gossip))
        .route("/system/bridge", post(peer_bridge))
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

    let session_dir = unique_test_dir("tachyon-bridge-steering-sessions");
    fs::create_dir_all(&session_dir).expect("session dir should exist");
    let client_a = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("client A should bind");
    let client_b = tokio::net::UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("client B should bind");
    let host_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("host listener should bind");
    let host_address = host_listener
        .local_addr()
        .expect("host listener should expose a local address");

    let mut bridge_route = system_targeted_route(SYSTEM_BRIDGE_ROUTE, "system-faas-bridge");
    let peer_urls = format!("http://{peer_address}");
    bridge_route.env = route_env(&[
        ("PEER_URLS", peer_urls.as_str()),
        ("BRIDGE_SOFT_LIMIT", "80"),
        ("GOSSIP_PATH", "/system/gossip"),
    ]);
    bridge_route.volumes = vec![mounted_ram_volume(&session_dir, "/sessions")];
    let config = validate_integrity_config(IntegrityConfig {
        host_address: host_address.to_string(),
        routes: vec![
            targeted_route(
                "/api/voip-gate",
                vec![weighted_target("guest-voip-gate", 100)],
            ),
            bridge_route,
        ],
        ..IntegrityConfig::default_sealed()
    })
    .expect("bridge steering config should validate");
    let state = build_test_state(config, telemetry::init_test_telemetry());

    let mut local_bridges = Vec::new();
    for offset in 0..4_u16 {
        let allocation = state
            .bridge_manager
            .create_relay(BridgeConfig {
                client_a_addr: format!("127.0.0.1:{}", 20_000 + offset * 2),
                client_b_addr: format!("127.0.0.1:{}", 20_001 + offset * 2),
                timeout_seconds: 30,
            })
            .expect("local saturation relay should allocate");
        local_bridges.push(allocation.bridge_id);
    }
    let local_relay_count = state.bridge_manager.active_relay_count();

    let app = build_app(state.clone());
    let server = tokio::spawn(async move {
        axum::serve(host_listener, app)
            .await
            .expect("bridge steering test app should stay up");
    });

    let allocation = Client::new()
            .post(format!("http://{host_address}/api/voip-gate"))
            .header("content-type", "application/json")
            .body(
                serde_json::to_vec(&serde_json::json!({
                    "client_a_addr": client_a.local_addr().expect("client A address should resolve").to_string(),
                    "client_b_addr": client_b.local_addr().expect("client B address should resolve").to_string(),
                    "timeout_seconds": 5
                }))
                .expect("voip gate request body should serialize"),
            )
            .send()
            .await
            .expect("voip gate request should succeed")
            .error_for_status()
            .expect("voip gate response should be OK")
            .bytes()
            .await
            .map(|body| {
                serde_json::from_slice::<BridgeAllocation>(&body)
                    .expect("bridge allocation response should decode")
            })
            .expect("voip gate response body should read");

    assert_eq!(allocation.bridge_id, "peer-bridge-1");
    assert_eq!(allocation.ip, "203.0.113.50");
    assert_eq!(allocation.port_a, 31_000);
    assert_eq!(allocation.port_b, 31_001);
    assert_eq!(state.bridge_manager.active_relay_count(), local_relay_count);

    {
        let capture = peer_capture
            .lock()
            .expect("peer capture should not be poisoned");
        assert_eq!(capture.bodies.len(), 1);
        assert!(capture.bodies[0].contains("client_a_addr"));
        assert!(capture.headers[0].iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("x-tachyon-bridge-delegated") && value == "true"
        }));
    }

    for bridge_id in local_bridges {
        state
            .bridge_manager
            .destroy_relay(&bridge_id)
            .expect("local saturation relay should tear down");
    }

    server.abort();
    let _ = server.await;
    peer_server.abort();
    let _ = peer_server.await;
    let _ = fs::remove_dir_all(session_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn https_listener_provisions_mock_certificate_once_and_serves_custom_domain() {
    init_host_tracing();
    let domain = "api.example.test";
    let cert_dir = unique_test_dir("tachyon-cert-manager-http");
    let mut route = IntegrityRoute::user("/api/guest-example");
    route.domains = vec![domain.to_owned()];
    let config = validate_integrity_config(IntegrityConfig {
        host_address: "127.0.0.1:8080".to_owned(),
        tls_address: Some("127.0.0.1:0".to_owned()),
        routes: vec![route, cert_manager_test_route(&cert_dir)],
        ..IntegrityConfig::default_sealed()
    })
    .expect("HTTPS config should validate");
    let state = build_test_state(config, telemetry::init_test_telemetry());
    let app = build_app(state.clone());
    let listener = start_https_listener(state.clone(), app)
        .await
        .expect("HTTPS listener should start")
        .expect("HTTPS listener should be enabled");
    let listener_addr = listener.local_addr;
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .resolve(domain, listener_addr)
        .build()
        .expect("reqwest HTTPS client should build");
    let url = format!("https://{domain}:{}/", listener_addr.port());

    let first = client
        .get(&url)
        .send()
        .await
        .expect("first HTTPS request should succeed");
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(
        first.text().await.expect("response body should decode"),
        expected_guest_example_body("FaaS received an empty payload")
    );
    assert_eq!(state.tls_manager.provision_count(), 1);
    assert!(
        cert_dir.join(format!("{domain}.json")).exists(),
        "cert-manager should persist issued material through the storage broker"
    );

    let second = client
        .get(&url)
        .send()
        .await
        .expect("cached HTTPS request should succeed");
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(state.tls_manager.provision_count(), 1);

    listener.join_handle.abort();
    let _ = listener.join_handle.await;
    let _ = fs::remove_dir_all(cert_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn mtls_gateway_rejects_missing_client_cert_and_forwards_authorized_requests() {
    init_host_tracing();

    let mtls = generate_mtls_test_material();
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("HTTP listener should bind");
    let http_address = http_listener
        .local_addr()
        .expect("HTTP listener should expose a local address");
    let mut config = IntegrityConfig::default_sealed();
    config.host_address = http_address.to_string();
    let mut gateway_route = IntegrityRoute::system(SYSTEM_GATEWAY_ROUTE);
    gateway_route.targets = vec![weighted_target("system-faas-gateway", 100)];
    config.routes.push(gateway_route);
    let config = validate_integrity_config(config).expect("mTLS gateway config should validate");
    let mut state = build_test_state(config, telemetry::init_test_telemetry());
    state.mtls_gateway = Some(Arc::new(tls_runtime::MtlsGatewayConfig {
        bind_address: "127.0.0.1:0"
            .parse()
            .expect("mTLS bind address should parse"),
        server_config: Arc::new(
            tls_runtime::build_mtls_server_config(
                &mtls.server_cert_pem,
                &mtls.server_key_pem,
                &mtls.ca_pem,
            )
            .expect("mTLS server config should build"),
        ),
    }));

    let app = build_app(state.clone());
    let http_server = tokio::spawn(async move {
        axum::serve(http_listener, app)
            .await
            .expect("HTTP app should stay up");
    });
    let listener = start_mtls_gateway_listener(state.clone())
        .await
        .expect("mTLS gateway listener should start")
        .expect("mTLS gateway listener should be enabled");
    let gateway_addr = listener.local_addr;
    let url = format!(
        "https://localhost:{}/api/guest-example",
        gateway_addr.port()
    );

    let unauthorized = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .expect("unauthorized reqwest client should build")
        .get(&url)
        .send()
        .await;
    assert!(
        unauthorized.is_err(),
        "mTLS gateway should reject requests without a client certificate"
    );

    let client_identity = reqwest::Identity::from_pem(
        format!("{}{}", mtls.client_cert_pem, mtls.client_key_pem).as_bytes(),
    )
    .expect("client identity should load");
    let authorized = reqwest::Client::builder()
        .use_rustls_tls()
        .danger_accept_invalid_certs(true)
        .identity(client_identity)
        .build()
        .expect("authorized reqwest client should build")
        .get(&url)
        .send()
        .await
        .expect("authorized mTLS request should succeed");
    assert_eq!(authorized.status(), StatusCode::OK);
    assert_eq!(
        authorized
            .text()
            .await
            .expect("authorized response should decode"),
        expected_guest_example_body("FaaS received an empty payload")
    );

    listener.join_handle.abort();
    let _ = listener.join_handle.await;
    http_server.abort();
    let _ = http_server.await;
}

#[cfg(feature = "http3")]
#[tokio::test(flavor = "multi_thread")]
async fn http3_listener_serves_guest_routes_over_quic() {
    use bytes::Buf;
    use h3::client;
    use quinn::crypto::rustls::QuicClientConfig;

    init_host_tracing();
    let domain = "api.example.test";
    let cert_dir = unique_test_dir("tachyon-cert-manager-http3");
    let mut route = IntegrityRoute::user("/api/guest-example");
    route.domains = vec![domain.to_owned()];
    let config = validate_integrity_config(IntegrityConfig {
        host_address: "127.0.0.1:8080".to_owned(),
        tls_address: Some("127.0.0.1:0".to_owned()),
        routes: vec![route, cert_manager_test_route(&cert_dir)],
        ..IntegrityConfig::default_sealed()
    })
    .expect("HTTP/3 config should validate");
    let state = build_test_state(config, telemetry::init_test_telemetry());
    let app = build_app(state.clone());
    let listener = start_http3_listener(state.clone(), app)
        .await
        .expect("HTTP/3 listener should start")
        .expect("HTTP/3 listener should be enabled");
    let listener_addr = listener.local_addr;

    let mut client_crypto = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])
    .expect("HTTP/3 client protocol versions should build")
    .dangerous()
    .with_custom_certificate_verifier(Arc::new(NoCertificateVerification))
    .with_no_client_auth();
    client_crypto.enable_early_data = true;
    client_crypto.alpn_protocols = vec![b"h3".to_vec()];
    let client_config = quinn::ClientConfig::new(Arc::new(
        QuicClientConfig::try_from(client_crypto).expect("HTTP/3 client config should convert"),
    ));
    let client_bind_addr = "127.0.0.1:0"
        .parse()
        .expect("literal HTTP/3 client bind address should parse");
    let mut endpoint =
        quinn::Endpoint::client(client_bind_addr).expect("HTTP/3 client endpoint should bind");
    endpoint.set_default_client_config(client_config);
    let connection = endpoint
        .connect(listener_addr, domain)
        .expect("HTTP/3 connect future should build")
        .await
        .expect("HTTP/3 handshake should succeed");

    let (mut driver, mut sender) = client::new(h3_quinn::Connection::new(connection.clone()))
        .await
        .expect("HTTP/3 client should initialize");
    let drive_task = tokio::spawn(async move {
        let _ = std::future::poll_fn(|cx| driver.poll_close(cx)).await;
    });
    let url = format!(
        "https://{domain}:{}/api/guest-example",
        listener_addr.port()
    );
    let mut request_stream = sender
        .send_request(
            Request::get(&url)
                .body(())
                .expect("HTTP/3 request should build"),
        )
        .await
        .expect("HTTP/3 request should send");
    request_stream
        .finish()
        .await
        .expect("HTTP/3 request body should finish");
    let response = request_stream
        .recv_response()
        .await
        .expect("HTTP/3 response head should arrive");
    assert_eq!(response.status(), StatusCode::OK);

    let mut body = Vec::new();
    while let Some(chunk) = request_stream
        .recv_data()
        .await
        .expect("HTTP/3 response body should stream")
    {
        let mut chunk = chunk;
        let bytes = chunk.copy_to_bytes(chunk.remaining());
        body.extend_from_slice(&bytes);
    }
    assert_eq!(
        String::from_utf8(body).expect("HTTP/3 response body should be UTF-8"),
        expected_guest_example_body("FaaS received an empty payload")
    );

    connection.close(0u32.into(), b"done");
    let _ = drive_task.await;
    listener.join_handle.abort();
    let _ = listener.join_handle.await;
    endpoint.wait_idle().await;
    let _ = fs::remove_dir_all(cert_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn tcp_layer4_listener_accepts_tls_when_route_declares_domains() {
    init_host_tracing();
    let domain = "echo.example.test";
    let cert_dir = unique_test_dir("tachyon-cert-manager-tcp");
    let port = free_tcp_port();
    let mut route = tcp_echo_test_route(1);
    route.domains = vec![domain.to_owned()];
    let config = validate_integrity_config(IntegrityConfig {
        host_address: "127.0.0.1:8080".to_owned(),
        layer4: IntegrityLayer4Config {
            tcp: vec![IntegrityTcpBinding {
                port,
                target: "guest-tcp-echo".to_owned(),
            }],
            udp: Vec::new(),
        },
        routes: vec![route, cert_manager_test_route(&cert_dir)],
        ..IntegrityConfig::default_sealed()
    })
    .expect("TLS Layer 4 config should validate");
    let state = build_test_state(config, telemetry::init_test_telemetry());
    let listeners = start_tcp_layer4_listeners(state.clone())
        .await
        .expect("TCP listener should start");
    let listener_addr = listeners
        .first()
        .expect("one TCP listener should be started")
        .local_addr;
    let connector = insecure_tls_connector();
    let tcp_stream = tokio::net::TcpStream::connect(listener_addr)
        .await
        .expect("TLS TCP client should connect");
    let server_name = ServerName::try_from(domain.to_owned()).expect("server name should be valid");
    let mut tls_stream = connector
        .connect(server_name, tcp_stream)
        .await
        .expect("TLS handshake should succeed");

    tls_stream
        .write_all(b"ping over tls")
        .await
        .expect("TLS client should write");
    tls_stream
        .shutdown()
        .await
        .expect("TLS client should close write side");

    let mut echoed = Vec::new();
    tls_stream
        .read_to_end(&mut echoed)
        .await
        .expect("TLS client should read echoed bytes");
    assert_eq!(echoed, b"ping over tls");
    assert_eq!(state.tls_manager.provision_count(), 1);

    for listener in listeners {
        listener.join_handle.abort();
        let _ = listener.join_handle.await;
    }
    let _ = fs::remove_dir_all(cert_dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn udp_layer4_listener_echoes_datagrams() {
    use std::time::{Duration, Instant};

    let port = free_udp_port();
    let route = udp_echo_test_route(1);
    let config = validate_integrity_config(IntegrityConfig {
        host_address: "127.0.0.1:8080".to_owned(),
        layer4: IntegrityLayer4Config {
            tcp: Vec::new(),
            udp: vec![IntegrityUdpBinding {
                port,
                target: "guest-udp-echo".to_owned(),
            }],
        },
        routes: vec![route],
        ..IntegrityConfig::default_sealed()
    })
    .expect("UDP Layer 4 config should validate");
    let state = build_test_state(config, telemetry::init_test_telemetry());
    let listeners = start_udp_layer4_listeners(state)
        .await
        .expect("UDP Layer 4 listener should start");
    let listener_addr = listeners
        .first()
        .expect("one UDP Layer 4 listener should be started")
        .local_addr;

    let client = std::net::UdpSocket::bind("127.0.0.1:0").expect("UDP client socket should bind");
    client
        .connect(listener_addr)
        .expect("UDP client should connect to listener");
    client
        .set_read_timeout(Some(Duration::from_millis(250)))
        .expect("UDP client should set a read timeout");
    client
        .send(b"ping over udp")
        .expect("UDP client should send datagram");

    let started = Instant::now();
    loop {
        let mut buffer = [0_u8; 64];
        match client.recv(&mut buffer) {
            Ok(received) => {
                assert_eq!(&buffer[..received], b"ping over udp");
                break;
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                assert!(
                    started.elapsed() <= Duration::from_secs(10),
                    "UDP client should receive echoed datagram before timing out"
                );
            }
            Err(error) => unreachable!("UDP client should receive echoed datagram: {error}"),
        }
    }

    for listener in listeners {
        for handle in listener.join_handles {
            handle.abort();
            let _ = handle.await;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn udp_layer4_listener_drops_when_safe_queue_is_full() {
    use std::time::{Duration, Instant};

    let port = free_udp_port();
    let route = udp_echo_test_route(1);
    let config = validate_integrity_config(IntegrityConfig {
        host_address: "127.0.0.1:8080".to_owned(),
        layer4: IntegrityLayer4Config {
            tcp: Vec::new(),
            udp: vec![IntegrityUdpBinding {
                port,
                target: "guest-udp-echo".to_owned(),
            }],
        },
        routes: vec![route],
        ..IntegrityConfig::default_sealed()
    })
    .expect("UDP Layer 4 config should validate");
    let state = build_test_state(config, telemetry::init_test_telemetry());
    let listeners = start_udp_layer4_listeners_with_queue_capacity(state, 1)
        .await
        .expect("UDP Layer 4 listener should start");
    let listener_addr = listeners
        .first()
        .expect("one UDP Layer 4 listener should be started")
        .local_addr;

    let client = std::net::UdpSocket::bind("127.0.0.1:0").expect("UDP client socket should bind");
    client
        .connect(listener_addr)
        .expect("UDP client should connect to listener");
    client
        .set_read_timeout(Some(Duration::from_millis(250)))
        .expect("UDP client should set a read timeout");

    client
        .send(b"delay:200")
        .expect("UDP client should send slow datagram");
    for index in 0..16 {
        let payload = format!("packet-{index}");
        client
            .send(payload.as_bytes())
            .expect("UDP client should send queued datagram");
    }

    let started = Instant::now();
    let mut responses = Vec::new();
    while started.elapsed() <= Duration::from_secs(10) {
        let mut buffer = [0_u8; 64];
        match client.recv(&mut buffer) {
            Ok(received) => {
                responses.push(String::from_utf8_lossy(&buffer[..received]).into_owned())
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    || error.kind() == std::io::ErrorKind::TimedOut =>
            {
                continue;
            }
            Err(error) => unreachable!("UDP client receive should not fail: {error}"),
        }
    }

    assert!(
        responses.iter().any(|payload| payload == "delay:200"),
        "the initially accepted datagram should complete"
    );
    assert!(
        responses.len() <= 2,
        "queue overload should drop excess datagrams, got {responses:?}"
    );

    for listener in listeners {
        for handle in listener.join_handles {
            handle.abort();
            let _ = handle.await;
        }
    }
}
