pub(crate) mod layer4 {
    #[cfg(feature = "experimental")]
    pub(crate) const MODULE: &str = "network::layer4";
}

pub(crate) mod layer7 {
    #[cfg(feature = "experimental")]
    pub(crate) const MODULE: &str = "network::layer7";
}

pub(crate) mod http3 {
    #[cfg(feature = "experimental")]
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
        // `Loaded` is only constructed inside the `ebpf-loader` feature build.
        // In default builds the variant is reserved but unreachable.
        #[cfg_attr(not(feature = "ebpf-loader"), allow(dead_code))]
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
    if runtime
        .route_registry
        .sealed_route(SYSTEM_GATEWAY_ROUTE)
        .is_none()
    {
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
    let Some(route) = runtime.route_registry.sealed_route(SYSTEM_GATEWAY_ROUTE) else {
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
    match execute_route_arc_with_middleware(
        &state,
        &runtime,
        route,
        &headers,
        &parts.method,
        &gateway_uri,
        &body,
        &trailers,
        HopLimit(DEFAULT_HOP_LIMIT),
        Some(&trace_id),
        false,
        None,
        true,
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
            .route_registry
            .sealed_route(&resolved.path)
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
            .route_registry
            .sealed_route(&resolved.path)
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
    route: Arc<IntegrityRoute>,
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
    let component_cache = Arc::clone(&runtime.component_cache);
    let component_instance_pre_cache = Arc::clone(&runtime.component_instance_pre_cache);
    let legacy_instance_pre_cache = Arc::clone(&runtime.legacy_instance_pre_cache);
    let linker_cache = Arc::clone(&runtime.linker_cache);
    let request_headers = HeaderMap::new();
    let route_for_execution = Arc::clone(&route);
    let route_overrides = Arc::clone(&state.route_overrides);
    let host_load = Arc::clone(&state.host_load);
    let source = datagram.source;
    let payload = datagram.payload;
    let responses = tokio::task::spawn_blocking(move || {
        let _volume_leases = volume_leases;
        let _permit = permit;
        let execution = GuestExecutionContext::builder(
            config.clone(),
            false,
            runtime_telemetry,
            state.async_log_sender.clone(),
            SecretAccess::from_route(&route_for_execution, &SecretsVault::load()),
            request_headers,
            host_identity,
            storage_broker,
            Arc::clone(&state.bridge_manager),
            concurrency_limits,
            Vec::new(),
            route_overrides,
            host_load,
            #[cfg(feature = "ai-inference")]
            Arc::clone(&runtime.ai_runtime),
        )
        .instance_pool(Some(instance_pool))
        .component_cache(Some(component_cache))
        .component_instance_pre_cache(Some(component_instance_pre_cache))
        .legacy_instance_pre_cache(Some(legacy_instance_pre_cache))
        .linker_cache(Some(linker_cache))
        .build();
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
    let local_mesh_dispatch = LocalMeshDispatchContext {
        state: state.clone(),
        runtime: Arc::clone(&runtime),
        handle: tokio::runtime::Handle::current(),
    };
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
    let component_cache = Arc::clone(&runtime.component_cache);
    let component_instance_pre_cache = Arc::clone(&runtime.component_instance_pre_cache);
    let legacy_instance_pre_cache = Arc::clone(&runtime.legacy_instance_pre_cache);
    let linker_cache = Arc::clone(&runtime.linker_cache);
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
        let execution = GuestExecutionContext::builder(
            config,
            false,
            runtime_telemetry,
            state.async_log_sender.clone(),
            secret_access,
            HeaderMap::new(),
            host_identity,
            storage_broker,
            Arc::clone(&state.bridge_manager),
            concurrency_limits,
            Vec::new(),
            route_overrides,
            host_load,
            #[cfg(feature = "ai-inference")]
            Arc::clone(&runtime.ai_runtime),
        )
        .instance_pool(Some(instance_pool))
        .component_cache(Some(component_cache))
        .component_instance_pre_cache(Some(component_instance_pre_cache))
        .legacy_instance_pre_cache(Some(legacy_instance_pre_cache))
        .linker_cache(Some(linker_cache))
        .local_mesh_dispatch(Some(local_mesh_dispatch))
        .build();
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

/// Handle a streaming HTTP request for user-role FaaS components. Mirrors
/// `handle_websocket_connection`: spawns a blocking thread to run the WASM
/// guest, wires up `tachyon:mesh/response-body` channels, and returns an
/// axum `Response` with a `GuestStreamingBody` body that drains live as the
/// guest produces chunks. Triggered by `Accept: text/event-stream` on
/// ai-inference routes.
///
/// Before running a local guest, this mirrors the two overflow checks
/// `execute_route_request` already applies for every other route shape —
/// RAM-pressure `AdmissionStrategy::MeshRetry` and a saturated
/// `RoutePermitError::TimedOut` with `allow_overflow` — so a genuine SSE
/// client (`Accept: text/event-stream`) can stream from a peer too, instead
/// of always either running locally or getting a bare 429. There is no
/// buffered-queue fallback here, matching this handler's pre-existing
/// behavior: a route that can't run locally and can't overflow to a peer
/// still just gets the timeout/429 it always got.
#[cfg(feature = "ai-inference")]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_streaming_http_request(
    state: AppState,
    runtime: Arc<RuntimeState>,
    route: Arc<IntegrityRoute>,
    function_name: String,
    request: GuestRequest,
    headers: &HeaderMap,
    method: &Method,
    hop_limit: HopLimit,
) -> std::result::Result<Response, (StatusCode, String)> {
    if let Some(result) = enforce_resource_admission(
        &state,
        &route,
        headers,
        method,
        &request.body,
        hop_limit,
        &runtime,
        true,
    )
    .await?
    {
        return Ok(guest_response_into_response(result));
    }

    let volume_leases = state
        .volume_manager
        .acquire_route_volumes(&route, Arc::clone(&state.storage_broker))
        .await
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to acquire streaming route volumes: {error}"),
            )
        })?;
    let semaphore = runtime
        .concurrency_limits
        .get(&route.path)
        .cloned()
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "streaming route `{}` is missing a concurrency limiter",
                    route.path
                ),
            )
        })?;
    let active_request_guard = semaphore.begin_request();
    let permit = match acquire_route_permit(semaphore).await {
        Ok(permit) => permit,
        Err(RoutePermitError::Closed) => {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                format!("streaming route `{}` is unavailable", route.path),
            ));
        }
        Err(RoutePermitError::TimedOut) => {
            if route.allow_overflow {
                let requested_model = requested_model_alias(&route, headers, &request.body);
                if let Some(destination) = control_plane_override_destination(
                    state.route_overrides.as_ref(),
                    &state.peer_capabilities,
                    &route.path,
                    headers,
                    select_route_target(&route, headers)
                        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?
                        .required_capability_mask,
                    requested_model.as_deref(),
                ) {
                    let idle_timeout = route
                        .resiliency
                        .as_ref()
                        .and_then(|resiliency| resiliency.timeout_ms)
                        .map(Duration::from_millis);
                    let forward = forward_request_to_override_as_streaming_response(
                        &state.http_client,
                        &destination,
                        headers,
                        method,
                        &request.body,
                        hop_limit,
                        idle_timeout,
                    );
                    // This call runs outside `execute_route_with_resiliency`'s
                    // `TimeoutLayer` entirely (this handler never goes
                    // through it), so nothing else bounds the wait for the
                    // peer to accept the connection and return headers. A
                    // peer that accepts but never responds would otherwise
                    // hang the request forever instead of honoring
                    // `resiliency.timeout_ms`.
                    let response = match idle_timeout {
                        Some(idle_timeout) => tokio::time::timeout(idle_timeout, forward)
                            .await
                            .map_err(|_| {
                            (
                                StatusCode::GATEWAY_TIMEOUT,
                                format!(
                                    "streaming route `{}` peer handshake timed out after {}ms",
                                    route.path,
                                    idle_timeout.as_millis()
                                ),
                            )
                        })??,
                        None => forward.await?,
                    };
                    return build_guest_response(response, None)
                        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error));
                }
            }
            return Err((
                StatusCode::TOO_MANY_REQUESTS,
                format!("streaming route `{}` is saturated", route.path),
            ));
        }
    };

    let engine = runtime.engine.clone();
    let config = runtime.config.clone();
    let runtime_telemetry = state.telemetry.clone();
    let secret_access = SecretAccess::from_route(&route, &state.secrets_vault);
    let host_identity = Arc::clone(&state.host_identity);
    let storage_broker = Arc::clone(&state.storage_broker);
    let concurrency_limits = Arc::clone(&runtime.concurrency_limits);
    let bridge_manager = Arc::clone(&state.bridge_manager);
    let route_overrides = Arc::clone(&state.route_overrides);
    let host_load = Arc::clone(&state.host_load);
    let ai_runtime = Arc::clone(&runtime.ai_runtime);
    let instance_pool = Arc::clone(&runtime.instance_pool);
    let component_cache = Arc::clone(&runtime.component_cache);
    let component_instance_pre_cache = Arc::clone(&runtime.component_instance_pre_cache);
    let legacy_instance_pre_cache = Arc::clone(&runtime.legacy_instance_pre_cache);
    let linker_cache = Arc::clone(&runtime.linker_cache);
    let async_log_sender = state.async_log_sender.clone();
    let request_headers = request
        .headers
        .iter()
        .filter_map(|(k, v)| {
            let name = HeaderName::from_bytes(k.as_bytes()).ok()?;
            let value = HeaderValue::from_str(v).ok()?;
            Some((name, value))
        })
        .collect::<HeaderMap>();

    let (headers_tx, headers_rx) = tokio::sync::oneshot::channel::<(StatusCode, GuestHttpFields)>();
    let (chunks_tx, chunks_rx) = tokio::sync::mpsc::channel::<Bytes>(32);
    // Cleared when axum drops the response body — the HTTP client hung up.
    //
    // The guest's own `token-stream` drop already reports a departed consumer,
    // but only once the guest gets control back: while it is parked inside
    // `next()` waiting on a silent upstream, nothing on that path can observe
    // the disconnect. Handing the same flag to the accelerator sink is what
    // lets the backend see it, so an upstream socket and its admission permit
    // are released when the client leaves rather than when the binding times
    // out.
    let consumer_alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let guest_consumer_alive = Arc::clone(&consumer_alive);

    std::thread::Builder::new()
        .name(format!("tachyon-streaming-{}", route.path))
        .spawn(move || {
            let _volume_leases = volume_leases;
            let _permit = permit;
            let execution = GuestExecutionContext::builder(
                config,
                false,
                runtime_telemetry,
                async_log_sender,
                secret_access,
                request_headers,
                host_identity,
                storage_broker,
                bridge_manager,
                concurrency_limits,
                Vec::new(),
                route_overrides,
                host_load,
                ai_runtime,
            )
            .instance_pool(Some(instance_pool))
            .component_cache(Some(component_cache))
            .component_instance_pre_cache(Some(component_instance_pre_cache))
            .legacy_instance_pre_cache(Some(legacy_instance_pre_cache))
            .linker_cache(Some(linker_cache))
            .build();
            execute_streaming_guest(
                &engine,
                &route,
                &function_name,
                request,
                headers_tx,
                chunks_tx,
                guest_consumer_alive,
                &execution,
            );
        })
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to spawn streaming guest thread: {e}"),
            )
        })?;

    // Wait for the guest to commit status + headers (via `begin()`) or for
    // `handle-request` to return (buffered fallback). Either way headers_rx
    // fires before any body bytes can be read.
    let (status, guest_headers) = headers_rx.await.map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "streaming guest exited without sending response headers".to_owned(),
        )
    })?;

    let body = GuestStreamingBody {
        receiver: chunks_rx,
        _completion_guard: Some(active_request_guard.into_response_guard()),
        consumer_alive,
    };

    let mut response = Response::builder()
        .status(status)
        .body(Body::new(body))
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to build streaming response: {e}"),
            )
        })?;
    if let Err(e) = insert_guest_fields(response.headers_mut(), &guest_headers, "response header") {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, e));
    }
    Ok(response)
}

pub(crate) async fn handle_tcp_layer4_connection(
    state: AppState,
    route: Arc<IntegrityRoute>,
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
    route: Arc<IntegrityRoute>,
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
    let execution = GuestExecutionContext::builder(
        config.clone(),
        false,
        runtime_telemetry,
        disconnected_log_sender(),
        SecretAccess::from_route(route, &SecretsVault::load()),
        HeaderMap::new(),
        host_identity,
        storage_broker,
        Arc::new(BridgeManager::default()),
        concurrency_limits,
        Vec::new(),
        route_overrides,
        host_load,
        #[cfg(feature = "ai-inference")]
        ai_runtime,
    )
    .build();
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
