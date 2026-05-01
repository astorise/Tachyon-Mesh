#![allow(dead_code)]

pub(crate) mod layer4 {
    pub(crate) const MODULE: &str = "network::layer4";
}

pub(crate) mod layer7 {
    pub(crate) const MODULE: &str = "network::layer7";
}

pub(crate) mod http3 {
    pub(crate) const MODULE: &str = "network::http3";
}

pub(crate) mod ebpf {
    #[cfg(all(target_os = "linux", feature = "ebpf-loader"))]
    use anyhow::{bail, Context};
    #[cfg(all(target_os = "linux", feature = "ebpf-loader"))]
    #[allow(deprecated)]
    use aya::{include_bytes_aligned, Bpf};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) enum EbpfFastPathStatus {
        Loaded,
        NoRules,
        Unsupported,
    }

    #[cfg(all(target_os = "linux", feature = "ebpf-loader"))]
    #[allow(deprecated)]
    pub(crate) fn load_ebpf_fast_path() -> anyhow::Result<Bpf> {
        let bpf_data = include_bytes_aligned!(concat!(env!("OUT_DIR"), "/tachyon-ebpf"));
        if option_env!("TACHYON_EBPF_ARTIFACT_PRESENT") != Some("1") {
            bail!(
                "compiled eBPF artifact not found at target/bpfel-unknown-none/release/tachyon-ebpf"
            );
        }

        let bpf = Bpf::load(bpf_data).context("failed to load Tachyon eBPF fast-path object")?;
        Ok(bpf)
    }

    pub(crate) fn init_ebpf_fastpath(route_count: usize) -> Result<EbpfFastPathStatus, String> {
        if route_count == 0 {
            return Ok(EbpfFastPathStatus::NoRules);
        }

        #[cfg(all(target_os = "linux", feature = "ebpf-loader"))]
        {
            let _bpf = load_ebpf_fast_path()
                .map_err(|error| format!("{error:#}; falling back to userspace L4 routing"))?;
            Ok(EbpfFastPathStatus::Loaded)
        }

        #[cfg(not(all(target_os = "linux", feature = "ebpf-loader")))]
        {
            Ok(EbpfFastPathStatus::Unsupported)
        }
    }
}

use super::*;

// Extracted HTTP listener loop.
pub(crate) async fn serve_http_listener(
    listener: tokio::net::TcpListener,
    app: Router,
) -> Result<()> {
    loop {
        let (stream, peer_addr) = listener
            .accept()
            .await
            .context("failed to accept HTTP connection")?;
        let service = app.clone();
        tokio::spawn(async move {
            let builder = HyperConnectionBuilder::new(TokioExecutor::new());
            let connection = builder.serve_connection_with_upgrades(
                TokioIo::new(stream),
                TowerToHyperService::new(service),
            );
            if let Err(error) = connection.await {
                tracing::warn!(remote = %peer_addr, "HTTP connection failed: {error}");
            }
        });
    }
}

// Extracted network listeners, TLS gateways, and L4 routing.

// Extracted network listeners, TLS gateways, and L4 routing.
#[cfg(unix)]
pub(crate) fn start_uds_fast_path_listener(
    app: Router,
    config: &IntegrityConfig,
    registry: Arc<UdsFastPathRegistry>,
) -> Result<Option<tokio::task::JoinHandle<()>>> {
    let listener = registry.bind_local_listener(config)?;
    let handle = tokio::spawn(async move {
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(accepted) => accepted,
                Err(error) => {
                    tracing::warn!("UDS fast-path listener accept failed: {error}");
                    break;
                }
            };

            let service = app.clone();
            tokio::spawn(async move {
                let builder = HyperConnectionBuilder::new(TokioExecutor::new());
                let connection = builder.serve_connection_with_upgrades(
                    TokioIo::new(stream),
                    TowerToHyperService::new(service),
                );
                if let Err(error) = connection.await {
                    tracing::warn!("UDS fast-path connection failed: {error}");
                }
            });
        }
    });

    Ok(Some(handle))
}

#[cfg(not(unix))]
pub(crate) fn start_uds_fast_path_listener(
    _app: Router,
    _config: &IntegrityConfig,
    _registry: Arc<UdsFastPathRegistry>,
) -> Result<Option<tokio::task::JoinHandle<()>>> {
    Ok(None)
}

pub(crate) fn layer4_bind_address(host_address: &str, port: u16) -> Result<SocketAddr> {
    let mut address = host_address.parse::<SocketAddr>().with_context(|| {
        format!("failed to parse `host_address` `{host_address}` for Layer 4 binding")
    })?;
    address.set_port(port);
    Ok(address)
}

pub(crate) fn https_bind_address(config: &IntegrityConfig) -> Result<Option<SocketAddr>> {
    if !config.has_custom_domains() {
        return Ok(None);
    }

    if let Some(address) = &config.tls_address {
        return address
            .parse()
            .with_context(|| format!("invalid tls_address `{address}`"))
            .map(Some);
    }

    let mut address = config.host_address.parse::<SocketAddr>().with_context(|| {
        format!(
            "failed to parse `host_address` `{}` for HTTPS binding",
            config.host_address
        )
    })?;
    address.set_port(443);
    Ok(Some(address))
}

pub(crate) async fn start_https_listener(
    state: AppState,
    app: Router,
) -> Result<Option<HttpsListenerHandle>> {
    let runtime = state.runtime.load_full();
    let Some(bind_address) = https_bind_address(&runtime.config)? else {
        return Ok(None);
    };

    let listener = tokio::net::TcpListener::bind(bind_address)
        .await
        .with_context(|| format!("failed to bind HTTPS listener on {bind_address}"))?;
    let local_addr = listener
        .local_addr()
        .context("failed to read HTTPS listener local address")?;

    let join_handle = tokio::spawn(async move {
        loop {
            let (stream, peer_addr) = match listener.accept().await {
                Ok(connection) => connection,
                Err(error) => {
                    tracing::warn!("HTTPS listener accept failed: {error}");
                    continue;
                }
            };
            let connection_state = state.clone();
            let connection_app = app.clone();
            tokio::spawn(async move {
                if let Err(error) =
                    handle_https_connection(connection_state, connection_app, stream).await
                {
                    tracing::warn!(remote = %peer_addr, "HTTPS connection failed: {error:#}");
                }
            });
        }
    });

    Ok(Some(HttpsListenerHandle {
        local_addr,
        join_handle,
    }))
}

pub(crate) async fn handle_https_connection(
    state: AppState,
    app: Router,
    stream: tokio::net::TcpStream,
) -> Result<()> {
    let start = LazyConfigAcceptor::new(tokio_rustls::rustls::server::Acceptor::default(), stream)
        .await
        .context("failed to accept TLS client hello")?;
    let client_hello = start.client_hello();
    let domain = client_hello
        .server_name()
        .ok_or_else(|| anyhow!("TLS client hello did not include SNI"))?;
    let config = state
        .tls_manager
        .server_config_for_domain(&state, domain)
        .await?;
    let tls_stream = start
        .into_stream(config)
        .await
        .context("failed to complete rustls handshake")?;

    HyperConnectionBuilder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(TokioIo::new(tls_stream), TowerToHyperService::new(app))
        .await
        .map_err(|error| anyhow!("HTTPS connection exited unexpectedly: {error}"))
}

pub(crate) async fn start_mtls_gateway_listener(
    state: AppState,
) -> Result<Option<MtlsGatewayListenerHandle>> {
    let Some(config) = state.mtls_gateway.as_ref().cloned() else {
        return Ok(None);
    };
    let runtime = state.runtime.load_full();
    if runtime.config.sealed_route(SYSTEM_GATEWAY_ROUTE).is_none() {
        return Ok(None);
    }

    let listener = tokio::net::TcpListener::bind(config.bind_address)
        .await
        .with_context(|| {
            format!(
                "failed to bind mTLS gateway listener on {}",
                config.bind_address
            )
        })?;
    let local_addr = listener
        .local_addr()
        .context("failed to read mTLS gateway listener local address")?;

    let join_handle = tokio::spawn(async move {
        loop {
            let (stream, peer_addr) = match listener.accept().await {
                Ok(connection) => connection,
                Err(error) => {
                    tracing::warn!("mTLS gateway listener accept failed: {error}");
                    continue;
                }
            };
            let connection_state = state.clone();
            let server_config = Arc::clone(&config.server_config);
            tokio::spawn(async move {
                if let Err(error) =
                    handle_mtls_gateway_connection(connection_state, server_config, stream).await
                {
                    tracing::warn!(remote = %peer_addr, "mTLS gateway connection failed: {error:#}");
                }
            });
        }
    });

    Ok(Some(MtlsGatewayListenerHandle {
        local_addr,
        join_handle,
    }))
}

pub(crate) async fn handle_mtls_gateway_connection(
    state: AppState,
    server_config: Arc<tokio_rustls::rustls::ServerConfig>,
    stream: tokio::net::TcpStream,
) -> Result<()> {
    let acceptor = tokio_rustls::TlsAcceptor::from(server_config);
    let tls_stream = acceptor
        .accept(stream)
        .await
        .context("failed to complete mTLS handshake")?;

    HyperConnectionBuilder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(
            TokioIo::new(tls_stream),
            service_fn(move |request| {
                let state = state.clone();
                async move {
                    Ok::<_, Infallible>(dispatch_mtls_gateway_request(state, request).await)
                }
            }),
        )
        .await
        .map_err(|error| anyhow!("mTLS gateway connection exited unexpectedly: {error}"))
}

pub(crate) async fn dispatch_mtls_gateway_request(
    state: AppState,
    request: hyper::Request<hyper::body::Incoming>,
) -> Response {
    let runtime = state.runtime.load_full();
    let Some(route) = runtime.config.sealed_route(SYSTEM_GATEWAY_ROUTE).cloned() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "sealed manifest does not define `/system/gateway`",
        )
            .into_response();
    };

    let (parts, body) = request.into_parts();
    let original_route = parts
        .uri
        .path_and_query()
        .map(|path| path.as_str().to_owned())
        .unwrap_or_else(|| parts.uri.path().to_owned());
    let body = match body.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("failed to read mTLS request body: {error}"),
            )
                .into_response();
        }
    };
    let mut headers = parts.headers;
    let original_route_value = match HeaderValue::from_str(&original_route) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("invalid original route header value `{original_route}`: {error}"),
            )
                .into_response();
        }
    };
    headers.insert(TACHYON_ORIGINAL_ROUTE_HEADER, original_route_value);

    let gateway_uri = Uri::from_static(SYSTEM_GATEWAY_ROUTE);
    let trailers = GuestHttpFields::new();
    let trace_id = Uuid::new_v4().to_string();
    match execute_route_with_middleware(
        &state,
        &runtime,
        &route,
        &headers,
        &parts.method,
        &gateway_uri,
        &body,
        &trailers,
        HopLimit(DEFAULT_HOP_LIMIT),
        Some(&trace_id),
        false,
        None,
    )
    .await
    {
        Ok(result) => guest_response_into_response(result),
        Err((status, message)) => (status, message).into_response(),
    }
}

#[cfg(feature = "http3")]
pub(crate) async fn start_http3_listener(
    state: AppState,
    app: Router,
) -> Result<Option<Http3ListenerHandle>> {
    server_h3::start_http3_listener(state, app).await
}

#[cfg(not(feature = "http3"))]
pub(crate) async fn start_http3_listener(
    _state: AppState,
    _app: Router,
) -> Result<Option<Http3ListenerHandle>> {
    Ok(None)
}

pub(crate) async fn start_udp_layer4_listeners(
    state: AppState,
) -> Result<Vec<UdpLayer4ListenerHandle>> {
    start_udp_layer4_listeners_with_queue_capacity(state, UDP_LAYER4_QUEUE_CAPACITY).await
}

pub(crate) async fn start_udp_layer4_listeners_with_queue_capacity(
    state: AppState,
    queue_capacity: usize,
) -> Result<Vec<UdpLayer4ListenerHandle>> {
    let runtime = state.runtime.load_full();
    let mut listeners = Vec::new();

    for binding in &runtime.config.layer4.udp {
        let resolved = runtime
            .route_registry
            .resolve_named_route(&binding.target)
            .map_err(|error| {
                anyhow!(
                    "invalid UDP Layer 4 binding target `{}`: {error}",
                    binding.target
                )
            })?;
        let route = runtime
            .config
            .sealed_route(&resolved.path)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "UDP Layer 4 binding target `{}` resolved to a missing route",
                    binding.target
                )
            })?;
        let bind_address = layer4_bind_address(&runtime.config.host_address, binding.port)?;
        let socket = Arc::new(
            tokio::net::UdpSocket::bind(bind_address)
                .await
                .with_context(|| {
                    format!("failed to bind UDP Layer 4 listener on {bind_address}")
                })?,
        );
        let local_addr = socket
            .local_addr()
            .context("failed to read bound UDP Layer 4 listener address")?;
        let (tx, rx) = mpsc::channel::<UdpInboundDatagram>(queue_capacity.max(1));
        let rx = Arc::new(TokioMutex::new(rx));
        let listener_socket = Arc::clone(&socket);
        let listener_target = binding.target.clone();
        let listener_handle = tokio::spawn(async move {
            let mut buffer = vec![0_u8; UDP_LAYER4_MAX_DATAGRAM_SIZE];
            loop {
                let (size, source) = match listener_socket.recv_from(&mut buffer).await {
                    Ok(received) => received,
                    Err(error) => {
                        tracing::warn!(
                            port = local_addr.port(),
                            target = listener_target,
                            "UDP Layer 4 listener receive failed: {error}"
                        );
                        break;
                    }
                };

                let packet = UdpInboundDatagram {
                    source,
                    payload: Bytes::copy_from_slice(&buffer[..size]),
                };
                if let Err(error) = tx.try_send(packet) {
                    match error {
                        mpsc::error::TrySendError::Full(_) => {
                            tracing::warn!(
                                port = local_addr.port(),
                                remote = %source,
                                target = listener_target,
                                "dropping UDP datagram because the safe queue threshold was exceeded"
                            );
                        }
                        mpsc::error::TrySendError::Closed(_) => break,
                    }
                }
            }
        });

        let mut join_handles = vec![listener_handle];
        for _ in 0..udp_listener_worker_count(route.max_concurrency) {
            let worker_state = state.clone();
            let worker_route = route.clone();
            let worker_socket = Arc::clone(&socket);
            let worker_rx = Arc::clone(&rx);
            let worker_target = binding.target.clone();
            join_handles.push(tokio::spawn(async move {
                loop {
                    let packet = {
                        let mut receiver = worker_rx.lock().await;
                        receiver.recv().await
                    };
                    let Some(packet) = packet else {
                        break;
                    };
                    if let Err(error) = handle_udp_layer4_datagram(
                        worker_state.clone(),
                        worker_route.clone(),
                        Arc::clone(&worker_socket),
                        packet,
                    )
                    .await
                    {
                        tracing::warn!(
                            target = %worker_target,
                            "UDP Layer 4 datagram failed: {error:#}"
                        );
                    }
                }
            }));
        }

        listeners.push(UdpLayer4ListenerHandle {
            local_addr,
            join_handles,
        });
    }

    Ok(listeners)
}

pub(crate) fn udp_listener_worker_count(max_concurrency: u32) -> usize {
    usize::try_from(max_concurrency)
        .ok()
        .map(|count| count.clamp(1, UDP_LAYER4_MAX_WORKERS_PER_LISTENER))
        .unwrap_or(UDP_LAYER4_MAX_WORKERS_PER_LISTENER)
}

pub(crate) async fn start_tcp_layer4_listeners(
    state: AppState,
) -> Result<Vec<TcpLayer4ListenerHandle>> {
    let runtime = state.runtime.load_full();
    let mut listeners = Vec::new();

    for binding in &runtime.config.layer4.tcp {
        let resolved = runtime
            .route_registry
            .resolve_named_route(&binding.target)
            .map_err(|error| {
                anyhow!(
                    "invalid TCP Layer 4 binding target `{}`: {error}",
                    binding.target
                )
            })?;
        let route = runtime
            .config
            .sealed_route(&resolved.path)
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "TCP Layer 4 binding target `{}` resolved to a missing route",
                    binding.target
                )
            })?;
        let bind_address = layer4_bind_address(&runtime.config.host_address, binding.port)?;
        let listener = tokio::net::TcpListener::bind(bind_address)
            .await
            .with_context(|| format!("failed to bind TCP Layer 4 listener on {bind_address}"))?;
        let local_addr = listener
            .local_addr()
            .context("failed to read bound TCP Layer 4 listener address")?;
        let listener_state = state.clone();
        let listener_route = route.clone();
        let listener_target = binding.target.clone();
        let join_handle = tokio::spawn(async move {
            loop {
                let (stream, remote_addr) = match listener.accept().await {
                    Ok(accepted) => accepted,
                    Err(error) => {
                        tracing::warn!(
                            port = local_addr.port(),
                            target = listener_target,
                            "TCP Layer 4 listener accept failed: {error}"
                        );
                        break;
                    }
                };

                let connection_state = listener_state.clone();
                let connection_route = listener_route.clone();
                let connection_target = connection_route.name.clone();
                tokio::spawn(async move {
                    if let Err(error) =
                        handle_tcp_layer4_connection(connection_state, connection_route, stream)
                            .await
                    {
                        tracing::warn!(
                            target = %connection_target,
                            remote = %remote_addr,
                            "TCP Layer 4 connection failed: {error:#}"
                        );
                    }
                });
            }
        });

        listeners.push(TcpLayer4ListenerHandle {
            local_addr,
            join_handle,
        });
    }

    Ok(listeners)
}

pub(crate) async fn handle_udp_layer4_datagram(
    state: AppState,
    route: IntegrityRoute,
    socket: Arc<tokio::net::UdpSocket>,
    datagram: UdpInboundDatagram,
) -> Result<()> {
    let runtime = state.runtime.load_full();
    let volume_leases = state
        .volume_manager
        .acquire_route_volumes(&route, Arc::clone(&state.storage_broker))
        .await
        .map_err(|error| anyhow!("failed to acquire UDP Layer 4 volumes: {error}"))?;
    let semaphore = runtime
        .concurrency_limits
        .get(&route.path)
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "UDP Layer 4 route `{}` is missing a concurrency limiter",
                route.path
            )
        })?;
    let permit = match acquire_route_permit(semaphore).await {
        Ok(permit) => permit,
        Err(RoutePermitError::Closed) => return Ok(()),
        Err(RoutePermitError::TimedOut) => {
            tracing::warn!(
                route = %route.path,
                remote = %datagram.source,
                "dropping UDP datagram because the route is saturated"
            );
            return Ok(());
        }
    };
    let function_name = select_stream_route_module(&route)
        .map_err(|error| anyhow!("failed to resolve UDP Layer 4 target module: {error}"))?;
    let engine = runtime.engine.clone();
    let config = runtime.config.clone();
    let runtime_telemetry = state.telemetry.clone();
    let host_identity = Arc::clone(&state.host_identity);
    let storage_broker = Arc::clone(&state.storage_broker);
    let concurrency_limits = Arc::clone(&runtime.concurrency_limits);
    let instance_pool = Arc::clone(&runtime.instance_pool);
    let request_headers = HeaderMap::new();
    let route_for_execution = route.clone();
    let route_overrides = Arc::clone(&state.route_overrides);
    let host_load = Arc::clone(&state.host_load);
    let source = datagram.source;
    let payload = datagram.payload;
    let responses = tokio::task::spawn_blocking(move || {
        let _volume_leases = volume_leases;
        let _permit = permit;
        let execution = GuestExecutionContext {
            config: config.clone(),
            sampled_execution: false,
            runtime_telemetry,
            async_log_sender: state.async_log_sender.clone(),
            secret_access: SecretAccess::from_route(&route_for_execution, &SecretsVault::load()),
            request_headers,
            host_identity,
            storage_broker,
            bridge_manager: Arc::clone(&state.bridge_manager),
            telemetry: None,
            concurrency_limits,
            propagated_headers: Vec::new(),
            route_overrides,
            host_load,
            #[cfg(feature = "ai-inference")]
            ai_runtime: Arc::clone(&runtime.ai_runtime),
            instance_pool: Some(instance_pool),
        };
        execute_udp_layer4_guest(
            &engine,
            &route_for_execution,
            &function_name,
            source,
            payload,
            &execution,
        )
    })
    .await
    .context("UDP Layer 4 worker exited before returning a result")?
    .map_err(|error| anyhow!("UDP Layer 4 guest failed: {error:?}"))?;

    for response in responses {
        socket
            .send_to(&response.payload, response.target)
            .await
            .with_context(|| format!("failed to send UDP datagram to {}", response.target))?;
    }

    Ok(())
}

#[cfg(feature = "websockets")]
pub(crate) async fn handle_websocket_connection(
    state: AppState,
    route: IntegrityRoute,
    function_name: String,
    socket: WebSocket,
) -> Result<()> {
    let runtime = state.runtime.load_full();
    let volume_leases = state
        .volume_manager
        .acquire_route_volumes(&route, Arc::clone(&state.storage_broker))
        .await
        .map_err(|error| anyhow!("failed to acquire WebSocket route volumes: {error}"))?;
    let semaphore = runtime
        .concurrency_limits
        .get(&route.path)
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "WebSocket route `{}` is missing a concurrency limiter",
                route.path
            )
        })?;
    let permit = acquire_route_permit(semaphore)
        .await
        .map_err(|error| match error {
            RoutePermitError::Closed => anyhow!("WebSocket route `{}` is unavailable", route.path),
            RoutePermitError::TimedOut => anyhow!("WebSocket route `{}` is saturated", route.path),
        })?;
    let engine = runtime.engine.clone();
    let config = runtime.config.clone();
    let runtime_telemetry = state.telemetry.clone();
    let host_identity = Arc::clone(&state.host_identity);
    let storage_broker = Arc::clone(&state.storage_broker);
    let concurrency_limits = Arc::clone(&runtime.concurrency_limits);
    let secret_access = SecretAccess::from_route(&route, &state.secrets_vault);
    let route_overrides = Arc::clone(&state.route_overrides);
    let host_load = Arc::clone(&state.host_load);
    let instance_pool = Arc::clone(&runtime.instance_pool);
    let (incoming_tx, incoming_rx) = std::sync::mpsc::channel::<HostWebSocketFrame>();
    let (outgoing_tx, mut outgoing_rx) =
        tokio::sync::mpsc::unbounded_channel::<HostWebSocketFrame>();
    let (mut writer, mut reader) = socket.split();

    let reader_handle = tokio::spawn(async move {
        while let Some(message) = reader.next().await {
            match message {
                Ok(message) => {
                    let frame = websocket_message_to_host_frame(message);
                    let should_close = matches!(frame, HostWebSocketFrame::Close);
                    if incoming_tx.send(frame).is_err() || should_close {
                        break;
                    }
                }
                Err(error) => {
                    tracing::warn!("WebSocket receive failed: {error}");
                    let _ = incoming_tx.send(HostWebSocketFrame::Close);
                    break;
                }
            }
        }
    });

    let writer_handle = tokio::spawn(async move {
        while let Some(frame) = outgoing_rx.recv().await {
            let should_close = matches!(frame, HostWebSocketFrame::Close);
            if writer
                .send(host_frame_to_websocket_message(frame))
                .await
                .is_err()
            {
                break;
            }
            if should_close {
                break;
            }
        }
        let _ = writer.close().await;
    });

    let (result_tx, result_rx) = oneshot::channel();
    std::thread::spawn(move || {
        let _volume_leases = volume_leases;
        let _permit = permit;
        let execution = GuestExecutionContext {
            config,
            sampled_execution: false,
            runtime_telemetry,
            async_log_sender: state.async_log_sender.clone(),
            secret_access,
            request_headers: HeaderMap::new(),
            host_identity,
            storage_broker,
            bridge_manager: Arc::clone(&state.bridge_manager),
            telemetry: None,
            concurrency_limits,
            propagated_headers: Vec::new(),
            route_overrides,
            host_load,
            #[cfg(feature = "ai-inference")]
            ai_runtime: Arc::clone(&runtime.ai_runtime),
            instance_pool: Some(instance_pool),
        };
        let _ = result_tx.send(execute_websocket_guest(
            &engine,
            &route,
            &function_name,
            incoming_rx,
            outgoing_tx,
            &execution,
        ));
    });

    let result = result_rx
        .await
        .context("WebSocket guest thread exited before returning a result")?;
    let _ = reader_handle.await;
    let _ = writer_handle.await;
    result.map_err(|error| anyhow!("WebSocket guest failed: {error:?}"))?;
    Ok(())
}

pub(crate) async fn handle_tcp_layer4_connection(
    state: AppState,
    route: IntegrityRoute,
    stream: tokio::net::TcpStream,
) -> Result<()> {
    let runtime = state.runtime.load_full();
    let volume_leases = state
        .volume_manager
        .acquire_route_volumes(&route, Arc::clone(&state.storage_broker))
        .await
        .map_err(|error| anyhow!("failed to acquire TCP Layer 4 volumes: {error}"))?;
    let semaphore = runtime
        .concurrency_limits
        .get(&route.path)
        .cloned()
        .ok_or_else(|| {
            anyhow!(
                "TCP Layer 4 route `{}` is missing a concurrency limiter",
                route.path
            )
        })?;
    let permit = acquire_route_permit(semaphore)
        .await
        .map_err(|error| match error {
            RoutePermitError::Closed => {
                anyhow!("TCP Layer 4 route `{}` is unavailable", route.path)
            }
            RoutePermitError::TimedOut => {
                anyhow!("TCP Layer 4 route `{}` is saturated", route.path)
            }
        })?;
    let function_name = select_stream_route_module(&route)
        .map_err(|error| anyhow!("failed to resolve TCP Layer 4 target module: {error}"))?;
    let engine = runtime.engine.clone();
    let config = runtime.config.clone();
    if !route.domains.is_empty() {
        return handle_tls_wrapped_tcp_layer4_connection(
            state,
            route,
            stream,
            function_name,
            engine,
            config,
            volume_leases,
            permit,
            runtime,
        )
        .await;
    }

    let socket = stream
        .into_std()
        .context("failed to convert TCP Layer 4 socket into std mode")?;
    socket
        .set_nonblocking(false)
        .context("failed to set TCP Layer 4 socket into blocking mode")?;
    let stdin_socket = socket
        .try_clone()
        .context("failed to clone TCP Layer 4 socket for guest stdin")?;
    let host_identity = Arc::clone(&state.host_identity);
    let storage_broker = Arc::clone(&state.storage_broker);
    let concurrency_limits = Arc::clone(&runtime.concurrency_limits);
    let telemetry = state.telemetry.clone();
    let route_overrides = Arc::clone(&state.route_overrides);
    let host_load = Arc::clone(&state.host_load);
    #[cfg(feature = "ai-inference")]
    let ai_runtime = Arc::clone(&runtime.ai_runtime);

    let (result_tx, result_rx) = oneshot::channel();
    std::thread::spawn(move || {
        let _volume_leases = volume_leases;
        let _permit = permit;
        let _ = result_tx.send(execute_tcp_layer4_guest(
            &engine,
            &config,
            &route,
            &function_name,
            TcpSocketStdin::new(stdin_socket),
            TcpSocketStdout::new(socket),
            telemetry,
            host_identity,
            storage_broker,
            concurrency_limits,
            route_overrides,
            host_load,
            #[cfg(feature = "ai-inference")]
            ai_runtime,
        ));
    });
    result_rx
        .await
        .context("TCP Layer 4 guest thread exited before returning a result")??;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_tls_wrapped_tcp_layer4_connection(
    state: AppState,
    route: IntegrityRoute,
    stream: tokio::net::TcpStream,
    function_name: String,
    engine: Engine,
    config: IntegrityConfig,
    volume_leases: RouteVolumeLeaseGuard,
    permit: OwnedSemaphorePermit,
    runtime: Arc<RuntimeState>,
) -> Result<()> {
    let start = LazyConfigAcceptor::new(tokio_rustls::rustls::server::Acceptor::default(), stream)
        .await
        .context("failed to accept TLS client hello for Layer 4 route")?;
    let client_hello = start.client_hello();
    let domain = tls_runtime::normalize_domain(
        client_hello
            .server_name()
            .ok_or_else(|| anyhow!("TLS Layer 4 client hello did not include SNI"))?,
    )?;
    if !route.domains.iter().any(|candidate| candidate == &domain) {
        return Err(anyhow!(
            "TLS Layer 4 route `{}` does not allow SNI `{domain}`",
            route.path
        ));
    }

    let tls_config = state
        .tls_manager
        .server_config_for_domain(&state, &domain)
        .await?;
    let mut tls_stream = start
        .into_stream(tls_config)
        .await
        .context("failed to complete TLS handshake for Layer 4 route")?;

    let bridge_listener = std::net::TcpListener::bind("127.0.0.1:0")
        .context("failed to bind local TLS bridge listener")?;
    let bridge_addr = bridge_listener
        .local_addr()
        .context("failed to resolve TLS bridge listener address")?;
    let host_identity = Arc::clone(&state.host_identity);
    let storage_broker = Arc::clone(&state.storage_broker);
    let concurrency_limits = Arc::clone(&runtime.concurrency_limits);
    let telemetry = state.telemetry.clone();
    #[cfg(feature = "ai-inference")]
    let ai_runtime = Arc::clone(&runtime.ai_runtime);

    let (result_tx, result_rx) = oneshot::channel();
    std::thread::spawn(move || {
        let _volume_leases = volume_leases;
        let _permit = permit;
        let result = (|| -> std::result::Result<(), ExecutionError> {
            let (socket, _) = bridge_listener.accept().map_err(|error| {
                guest_execution_error(error.into(), "failed to accept TLS bridge socket")
            })?;
            let stdin_socket = socket.try_clone().map_err(|error| {
                guest_execution_error(error.into(), "failed to clone TLS bridge socket")
            })?;
            execute_tcp_layer4_guest(
                &engine,
                &config,
                &route,
                &function_name,
                TcpSocketStdin::new(stdin_socket),
                TcpSocketStdout::new(socket),
                telemetry,
                host_identity,
                storage_broker,
                concurrency_limits,
                Arc::clone(&state.route_overrides),
                Arc::clone(&state.host_load),
                #[cfg(feature = "ai-inference")]
                ai_runtime,
            )
        })();
        let _ = result_tx.send(result);
    });

    let mut bridge_stream = tokio::net::TcpStream::connect(bridge_addr)
        .await
        .context("failed to connect local TLS bridge stream")?;
    tokio::io::copy_bidirectional(&mut tls_stream, &mut bridge_stream)
        .await
        .context("failed to proxy decrypted TLS Layer 4 stream")?;

    result_rx
        .await
        .context("TLS Layer 4 guest thread exited before returning a result")??;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn execute_tcp_layer4_guest(
    engine: &Engine,
    config: &IntegrityConfig,
    route: &IntegrityRoute,
    function_name: &str,
    stdin_stream: TcpSocketStdin,
    stdout_stream: TcpSocketStdout,
    runtime_telemetry: TelemetryHandle,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
    route_overrides: Arc<ArcSwap<HashMap<String, String>>>,
    host_load: Arc<HostLoadCounters>,
    #[cfg(feature = "ai-inference")] ai_runtime: Arc<ai_inference::AiInferenceRuntime>,
) -> std::result::Result<(), ExecutionError> {
    let execution = GuestExecutionContext {
        config: config.clone(),
        sampled_execution: false,
        runtime_telemetry,
        async_log_sender: disconnected_log_sender(),
        secret_access: SecretAccess::from_route(route, &SecretsVault::load()),
        request_headers: HeaderMap::new(),
        host_identity,
        storage_broker,
        bridge_manager: Arc::new(BridgeManager::default()),
        telemetry: None,
        concurrency_limits,
        propagated_headers: Vec::new(),
        route_overrides,
        host_load,
        #[cfg(feature = "ai-inference")]
        ai_runtime,
        instance_pool: None,
    };
    let (module_path, module) = resolve_legacy_guest_module_with_pool(
        engine,
        function_name,
        &execution.storage_broker.core_store,
        "default",
        execution.instance_pool.as_deref(),
    )?;
    execute_legacy_guest_with_stdio(
        engine,
        route,
        &module_path,
        module,
        &execution,
        stdin_stream,
        stdout_stream,
    )
}
