fn execute_guest(
    engine: &Engine,
    function_name: &str,
    request: GuestRequest,
    route: &IntegrityRoute,
    execution: GuestExecutionContext,
) -> std::result::Result<GuestExecutionOutcome, ExecutionError> {
    #[cfg(not(feature = "ai-inference"))]
    if requires_ai_inference_feature(function_name) {
        return Err(ExecutionError::Internal(format!(
            "guest `{function_name}` requires `core-host` to be built with `--features ai-inference`"
        )));
    }

    let module_path =
        resolve_guest_module_path(function_name).map_err(ExecutionError::GuestModuleNotFound)?;
    let cache_scope = if execution.sampled_execution {
        "metered"
    } else {
        "default"
    };

    if let Ok(component) = load_component_with_core_store(
        engine,
        &module_path,
        &execution.storage_broker.core_store,
        cache_scope,
    ) {
        let component_result = match route.role {
            RouteRole::User => execute_component_guest(
                engine,
                request.clone(),
                route,
                &module_path,
                &component,
                &execution,
            ),
            RouteRole::System => execute_system_component_guest(
                engine,
                request.clone(),
                route,
                &module_path,
                &component,
                &execution,
            ),
        };

        match component_result {
            Ok(response) => return Ok(response),
            Err(ExecutionError::Internal(message))
                if message.contains("no exported instance named `tachyon:mesh/handler`") => {}
            Err(error) => return Err(error),
        }
    }

    let (module_path, module) = resolve_legacy_guest_module_with_pool(
        engine,
        function_name,
        &execution.storage_broker.core_store,
        cache_scope,
        execution.instance_pool.as_deref(),
    )?;

    execute_legacy_guest(
        engine,
        function_name,
        request.body,
        route,
        &module_path,
        module,
        &execution,
    )
}

#[derive(Clone, Copy)]
enum CompiledArtifactKind {
    Component,
    Module,
}

fn load_component_with_core_store(
    engine: &Engine,
    module_path: &Path,
    core_store: &store::CoreStore,
    cache_scope: &str,
) -> Result<Component> {
    let wasm_bytes = fs::read(module_path).with_context(|| {
        format!(
            "failed to read guest component artifact from {}",
            module_path.display()
        )
    })?;
    let cache_key = compiled_artifact_cache_key(
        engine,
        module_path,
        &wasm_bytes,
        CompiledArtifactKind::Component,
        cache_scope,
    );

    if let Some(cached) = core_store
        .get(store::CoreStoreBucket::CwasmCache, &cache_key)
        .with_context(|| {
            format!(
                "failed to read cached component `{}`",
                module_path.display()
            )
        })?
    {
        // SAFETY: cached bytes originate from Engine::precompile_component for this host.
        if let Ok(component) = unsafe { Component::deserialize(engine, &cached) } {
            return Ok(component);
        }
    }

    let compiled = engine.precompile_component(&wasm_bytes).map_err(|error| {
        anyhow!(
            "failed to precompile guest component artifact from {}: {error}",
            module_path.display()
        )
    })?;
    core_store
        .put(store::CoreStoreBucket::CwasmCache, &cache_key, &compiled)
        .with_context(|| format!("failed to cache component `{}`", module_path.display()))?;
    // SAFETY: compiled bytes were produced by Engine::precompile_component above.
    unsafe { Component::deserialize(engine, &compiled) }.map_err(|error| {
        anyhow!(
            "failed to deserialize cached guest component from {}: {error}",
            module_path.display()
        )
    })
}

#[cfg(test)]
fn resolve_legacy_guest_module(
    engine: &Engine,
    function_name: &str,
    core_store: &store::CoreStore,
    cache_scope: &str,
) -> std::result::Result<(PathBuf, Module), ExecutionError> {
    resolve_legacy_guest_module_with_pool(engine, function_name, core_store, cache_scope, None)
}

/// Same as `resolve_legacy_guest_module`, but consults the runtime's in-memory
/// instance pool first. The pool stores `Arc<Module>` keyed by the resolved
/// canonical path; on a hit we skip the redb lookup and the
/// `Module::deserialize` cost entirely. On a miss we load through the existing
/// redb-backed precompile path and populate the pool for subsequent requests.
fn resolve_legacy_guest_module_with_pool(
    engine: &Engine,
    function_name: &str,
    core_store: &store::CoreStore,
    cache_scope: &str,
    instance_pool: Option<&moka::sync::Cache<PathBuf, Arc<Module>>>,
) -> std::result::Result<(PathBuf, Module), ExecutionError> {
    let candidates = guest_module_candidate_paths(function_name);
    let candidate_strings = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    let mut last_error = None;

    for candidate in candidates {
        if !candidate.exists() {
            continue;
        }

        let normalized = normalize_path(candidate.clone());
        if let Some(pool) = instance_pool {
            if let Some(cached) = pool.get(&normalized) {
                // `Module` is internally Arc-backed; cloning the `Module` value out
                // of the `Arc<Module>` returned by the pool is cheap.
                return Ok((normalized, (*cached).clone()));
            }
        }

        match load_module_with_core_store(engine, &candidate, core_store, cache_scope) {
            Ok(module) => {
                if let Some(pool) = instance_pool {
                    pool.insert(normalized.clone(), Arc::new(module.clone()));
                }
                return Ok((normalized, module));
            }
            Err(error) => last_error = Some((normalized, error)),
        }
    }

    if let Some((path, error)) = last_error {
        return Err(ExecutionError::Internal(format!(
            "failed to load guest artifact from {}: {error:#}",
            path.display()
        )));
    }

    Err(ExecutionError::GuestModuleNotFound(
        GuestModuleNotFound::new(function_name, format_candidate_list(&candidate_strings)),
    ))
}

fn load_module_with_core_store(
    engine: &Engine,
    module_path: &Path,
    core_store: &store::CoreStore,
    cache_scope: &str,
) -> Result<Module> {
    let wasm_bytes = fs::read(module_path).with_context(|| {
        format!(
            "failed to read guest module artifact from {}",
            module_path.display()
        )
    })?;
    let cache_key = compiled_artifact_cache_key(
        engine,
        module_path,
        &wasm_bytes,
        CompiledArtifactKind::Module,
        cache_scope,
    );

    if let Some(cached) = core_store
        .get(store::CoreStoreBucket::CwasmCache, &cache_key)
        .with_context(|| format!("failed to read cached module `{}`", module_path.display()))?
    {
        // SAFETY: cached bytes originate from Engine::precompile_module for this host.
        if let Ok(module) = unsafe { Module::deserialize(engine, &cached) } {
            return Ok(module);
        }
    }

    let compiled = engine.precompile_module(&wasm_bytes).map_err(|error| {
        anyhow!(
            "failed to precompile guest module artifact from {}: {error}",
            module_path.display()
        )
    })?;
    core_store
        .put(store::CoreStoreBucket::CwasmCache, &cache_key, &compiled)
        .with_context(|| format!("failed to cache module `{}`", module_path.display()))?;
    // SAFETY: compiled bytes were produced by Engine::precompile_module above.
    unsafe { Module::deserialize(engine, &compiled) }.map_err(|error| {
        anyhow!(
            "failed to deserialize cached guest module from {}: {error}",
            module_path.display()
        )
    })
}

fn compiled_artifact_cache_key(
    engine: &Engine,
    module_path: &Path,
    wasm_bytes: &[u8],
    kind: CompiledArtifactKind,
    cache_scope: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(wasm_bytes);
    let digest = hasher.finalize();
    let kind = match kind {
        CompiledArtifactKind::Component => "component",
        CompiledArtifactKind::Module => "module",
    };

    format!(
        "{kind}:{cache_scope}:{}:{}:{}",
        module_path.display(),
        hex::encode(digest),
        engine_precompile_hash_string(engine)
    )
}

fn execute_component_guest(
    engine: &Engine,
    request: GuestRequest,
    route: &IntegrityRoute,
    component_path: &Path,
    component: &Component,
    execution: &GuestExecutionContext,
) -> std::result::Result<GuestExecutionOutcome, ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to component linker",
        )
    })?;
    component_bindings::tachyon::mesh::secrets_vault::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add secrets vault functions to component linker",
        )
    })?;
    component_bindings::tachyon::mesh::bridge_controller::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add bridge controller functions to component linker",
        )
    })?;
    component_bindings::tachyon::mesh::vector::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add vector store functions to component linker",
        )
    })?;
    component_bindings::tachyon::mesh::training::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add training functions to component linker",
        )
    })?;
    #[cfg(feature = "ai-inference")]
    add_accelerator_interfaces_to_component_linker(
        &mut linker,
        execution.ai_runtime.as_ref(),
        "component linker",
    )?;
    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            execution.config.clone(),
            execution.config.guest_memory_limit_bytes,
            execution.runtime_telemetry.clone(),
            execution.secret_access.clone(),
            execution.request_headers.clone(),
            Arc::clone(&execution.host_identity),
            Arc::clone(&execution.storage_broker),
            Arc::clone(&execution.concurrency_limits),
            execution.propagated_headers.clone(),
        )?,
    );
    #[cfg(feature = "ai-inference")]
    {
        store.data_mut().ai_runtime = Some(Arc::clone(&execution.ai_runtime));
    }
    store.data_mut().bridge_manager = Arc::clone(&execution.bridge_manager);
    store.limiter(|state| &mut state.limits);
    maybe_set_guest_fuel_budget(&mut store, execution)?;

    let bindings = component_bindings::FaasGuest::instantiate(&mut store, component, &linker)
        .map_err(|error| {
            guest_execution_error(
                error,
                format!(
                    "failed to instantiate guest component from {}",
                    component_path.display()
                ),
            )
        })?;
    record_wasm_start(execution.telemetry.as_ref());
    let response = bindings.tachyon_mesh_handler().call_handle_request(
        &mut store,
        &component_bindings::exports::tachyon::mesh::handler::Request {
            method: request.method,
            uri: request.uri,
            headers: request.headers,
            body: request.body.to_vec(),
            trailers: request.trailers,
        },
    );
    record_wasm_end(execution.telemetry.as_ref());
    let fuel_consumed = sampled_fuel_consumed(&mut store, execution)?;
    let response = response.map_err(|error| {
        guest_execution_error(error, "guest component `handle-request` trapped")
    })?;
    let status = StatusCode::from_u16(response.status).map_err(|error| {
        ExecutionError::Internal(format!(
            "guest component returned an invalid HTTP status code `{}`: {error}",
            response.status
        ))
    })?;

    Ok(GuestExecutionOutcome {
        output: GuestExecutionOutput::Http(GuestHttpResponse {
            status,
            headers: response.headers,
            body: Bytes::from(response.body),
            trailers: response.trailers,
        }),
        fuel_consumed,
    })
}

fn execute_udp_layer4_guest(
    engine: &Engine,
    route: &IntegrityRoute,
    function_name: &str,
    source: SocketAddr,
    payload: Bytes,
    execution: &GuestExecutionContext,
) -> std::result::Result<Vec<UdpResponseDatagram>, ExecutionError> {
    let module_path =
        resolve_guest_module_path(function_name).map_err(ExecutionError::GuestModuleNotFound)?;
    let component = load_component_with_core_store(
        engine,
        &module_path,
        &execution.storage_broker.core_store,
        "default",
    )
    .map_err(|error| {
        ExecutionError::Internal(format!(
            "failed to load UDP guest component from {}: {error:#}",
            module_path.display()
        ))
    })?;

    execute_udp_component_guest(
        engine,
        route,
        &module_path,
        &component,
        source,
        payload,
        execution,
    )
}

fn execute_udp_component_guest(
    engine: &Engine,
    route: &IntegrityRoute,
    component_path: &Path,
    component: &Component,
    source: SocketAddr,
    payload: Bytes,
    execution: &GuestExecutionContext,
) -> std::result::Result<Vec<UdpResponseDatagram>, ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to UDP component linker",
        )
    })?;
    udp_component_bindings::tachyon::mesh::secrets_vault::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add secrets vault functions to UDP component linker",
        )
    })?;
    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            execution.config.clone(),
            execution.config.guest_memory_limit_bytes,
            execution.runtime_telemetry.clone(),
            execution.secret_access.clone(),
            execution.request_headers.clone(),
            Arc::clone(&execution.host_identity),
            Arc::clone(&execution.storage_broker),
            Arc::clone(&execution.concurrency_limits),
            execution.propagated_headers.clone(),
        )?,
    );
    store.limiter(|state| &mut state.limits);
    maybe_set_guest_fuel_budget(&mut store, execution)?;

    let bindings = udp_component_bindings::UdpFaasGuest::instantiate(
        &mut store, component, &linker,
    )
    .map_err(|error| {
        let message = format!(
            "failed to instantiate UDP guest component from {}",
            component_path.display()
        );
        let error_message = error.to_string();
        if error_message.contains("no exported instance named `tachyon:mesh/udp-handler`") {
            ExecutionError::Internal(format!(
                "guest component `{}` does not export the UDP packet handler",
                component_path.display()
            ))
        } else {
            guest_execution_error(error, message)
        }
    })?;
    record_wasm_start(execution.telemetry.as_ref());
    let source_ip = source.ip().to_string();
    let response = bindings.tachyon_mesh_udp_handler().call_handle_packet(
        &mut store,
        &source_ip,
        source.port(),
        payload.as_ref(),
    );
    record_wasm_end(execution.telemetry.as_ref());
    let _fuel_consumed = sampled_fuel_consumed(&mut store, execution)?;
    let response = response.map_err(|error| {
        guest_execution_error(error, "guest UDP component `handle-packet` trapped")
    })?;

    response
        .into_iter()
        .map(|datagram| {
            let target_ip = datagram.target_ip.parse::<IpAddr>().map_err(|error| {
                ExecutionError::Internal(format!(
                    "guest UDP component returned an invalid target IP `{}`: {error}",
                    datagram.target_ip
                ))
            })?;
            Ok(UdpResponseDatagram {
                target: SocketAddr::new(target_ip, datagram.target_port),
                payload: Bytes::from(datagram.payload),
            })
        })
        .collect()
}

#[cfg(feature = "websockets")]
fn execute_websocket_guest(
    engine: &Engine,
    route: &IntegrityRoute,
    function_name: &str,
    incoming: std::sync::mpsc::Receiver<HostWebSocketFrame>,
    outgoing: tokio::sync::mpsc::UnboundedSender<HostWebSocketFrame>,
    execution: &GuestExecutionContext,
) -> std::result::Result<(), ExecutionError> {
    let module_path =
        resolve_guest_module_path(function_name).map_err(ExecutionError::GuestModuleNotFound)?;
    let component = load_component_with_core_store(
        engine,
        &module_path,
        &execution.storage_broker.core_store,
        "default",
    )
    .map_err(|error| {
        ExecutionError::Internal(format!(
            "failed to load WebSocket guest component from {}: {error:#}",
            module_path.display()
        ))
    })?;

    execute_websocket_component_guest(
        engine,
        route,
        &module_path,
        &component,
        incoming,
        outgoing,
        execution,
    )
}

#[cfg(feature = "websockets")]
fn execute_websocket_component_guest(
    engine: &Engine,
    route: &IntegrityRoute,
    component_path: &Path,
    component: &Component,
    incoming: std::sync::mpsc::Receiver<HostWebSocketFrame>,
    outgoing: tokio::sync::mpsc::UnboundedSender<HostWebSocketFrame>,
    execution: &GuestExecutionContext,
) -> std::result::Result<(), ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to WebSocket component linker",
        )
    })?;
    websocket_component_bindings::tachyon::mesh::secrets_vault::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add secrets vault functions to WebSocket component linker",
        )
    })?;
    websocket_component_bindings::tachyon::mesh::websocket::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WebSocket host functions to component linker",
        )
    })?;
    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            execution.config.clone(),
            execution.config.guest_memory_limit_bytes,
            execution.runtime_telemetry.clone(),
            execution.secret_access.clone(),
            execution.request_headers.clone(),
            Arc::clone(&execution.host_identity),
            Arc::clone(&execution.storage_broker),
            Arc::clone(&execution.concurrency_limits),
            execution.propagated_headers.clone(),
        )?,
    );
    store.limiter(|state| &mut state.limits);
    maybe_set_guest_fuel_budget(&mut store, execution)?;

    let stored_connection = store
        .data_mut()
        .table
        .push(HostWebSocketConnection { incoming, outgoing })
        .map_err(|error| {
            ExecutionError::Internal(format!(
                "failed to store WebSocket connection resource for {}: {error}",
                component_path.display()
            ))
        })?;
    let connection = wasmtime::component::Resource::<
        websocket_component_bindings::tachyon::mesh::websocket::Connection,
    >::new_own(stored_connection.rep());

    let bindings = websocket_component_bindings::WebsocketFaasGuest::instantiate(
        &mut store, component, &linker,
    )
    .map_err(|error| {
        let message = format!(
            "failed to instantiate WebSocket guest component from {}",
            component_path.display()
        );
        let error_message = error.to_string();
        if error_message.contains("on-connect") {
            ExecutionError::Internal(format!(
                "guest component `{}` does not export the WebSocket `on-connect` handler",
                component_path.display()
            ))
        } else {
            guest_execution_error(error, message)
        }
    })?;
    record_wasm_start(execution.telemetry.as_ref());
    let result = bindings.call_on_connect(&mut store, connection);
    record_wasm_end(execution.telemetry.as_ref());
    let _fuel_consumed = sampled_fuel_consumed(&mut store, execution)?;
    result.map_err(|error| {
        guest_execution_error(error, "guest WebSocket component `on-connect` trapped")
    })?;
    Ok(())
}

#[cfg(feature = "websockets")]
fn websocket_message_to_host_frame(message: AxumWebSocketMessage) -> HostWebSocketFrame {
    match message {
        AxumWebSocketMessage::Text(text) => HostWebSocketFrame::Text(text.to_string()),
        AxumWebSocketMessage::Binary(bytes) => HostWebSocketFrame::Binary(bytes.to_vec()),
        AxumWebSocketMessage::Ping(bytes) => HostWebSocketFrame::Ping(bytes.to_vec()),
        AxumWebSocketMessage::Pong(bytes) => HostWebSocketFrame::Pong(bytes.to_vec()),
        AxumWebSocketMessage::Close(_) => HostWebSocketFrame::Close,
    }
}

#[cfg(feature = "websockets")]
fn host_frame_to_websocket_message(frame: HostWebSocketFrame) -> AxumWebSocketMessage {
    match frame {
        HostWebSocketFrame::Text(text) => AxumWebSocketMessage::Text(text.into()),
        HostWebSocketFrame::Binary(bytes) => AxumWebSocketMessage::Binary(bytes.into()),
        HostWebSocketFrame::Ping(bytes) => AxumWebSocketMessage::Ping(bytes.into()),
        HostWebSocketFrame::Pong(bytes) => AxumWebSocketMessage::Pong(bytes.into()),
        HostWebSocketFrame::Close => AxumWebSocketMessage::Close(None),
    }
}

#[cfg(feature = "websockets")]
fn websocket_binding_frame_to_host_frame(
    frame: websocket_component_bindings::tachyon::mesh::websocket::Frame,
) -> HostWebSocketFrame {
    match frame {
        websocket_component_bindings::tachyon::mesh::websocket::Frame::Text(text) => {
            HostWebSocketFrame::Text(text)
        }
        websocket_component_bindings::tachyon::mesh::websocket::Frame::Binary(bytes) => {
            HostWebSocketFrame::Binary(bytes)
        }
        websocket_component_bindings::tachyon::mesh::websocket::Frame::Ping(bytes) => {
            HostWebSocketFrame::Ping(bytes)
        }
        websocket_component_bindings::tachyon::mesh::websocket::Frame::Pong(bytes) => {
            HostWebSocketFrame::Pong(bytes)
        }
        websocket_component_bindings::tachyon::mesh::websocket::Frame::Close => {
            HostWebSocketFrame::Close
        }
    }
}

#[cfg(feature = "websockets")]
fn host_frame_to_websocket_binding_frame(
    frame: HostWebSocketFrame,
) -> websocket_component_bindings::tachyon::mesh::websocket::Frame {
    match frame {
        HostWebSocketFrame::Text(text) => {
            websocket_component_bindings::tachyon::mesh::websocket::Frame::Text(text)
        }
        HostWebSocketFrame::Binary(bytes) => {
            websocket_component_bindings::tachyon::mesh::websocket::Frame::Binary(bytes)
        }
        HostWebSocketFrame::Ping(bytes) => {
            websocket_component_bindings::tachyon::mesh::websocket::Frame::Ping(bytes)
        }
        HostWebSocketFrame::Pong(bytes) => {
            websocket_component_bindings::tachyon::mesh::websocket::Frame::Pong(bytes)
        }
        HostWebSocketFrame::Close => {
            websocket_component_bindings::tachyon::mesh::websocket::Frame::Close
        }
    }
}

fn execute_system_component_guest(
    engine: &Engine,
    request: GuestRequest,
    route: &IntegrityRoute,
    component_path: &Path,
    component: &Component,
    execution: &GuestExecutionContext,
) -> std::result::Result<GuestExecutionOutcome, ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to system component linker",
        )
    })?;
    control_plane_component_bindings::tachyon::mesh::telemetry_reader::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add telemetry reader functions to system component linker",
        )
    })?;
    control_plane_component_bindings::tachyon::mesh::scaling_metrics::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add scaling metrics functions to system component linker",
        )
    })?;
    control_plane_component_bindings::tachyon::mesh::outbound_http::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add outbound HTTP functions to system component linker",
        )
    })?;
    control_plane_component_bindings::tachyon::mesh::routing_control::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add routing control functions to system component linker",
        )
    })?;
    system_component_bindings::tachyon::mesh::bridge_controller::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add bridge controller functions to system component linker",
        )
    })?;
    system_component_bindings::tachyon::mesh::storage_broker::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add storage broker functions to system component linker",
        )
    })?;

    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            execution.config.clone(),
            execution.config.guest_memory_limit_bytes,
            execution.runtime_telemetry.clone(),
            execution.secret_access.clone(),
            execution.request_headers.clone(),
            Arc::clone(&execution.host_identity),
            Arc::clone(&execution.storage_broker),
            Arc::clone(&execution.concurrency_limits),
            execution.propagated_headers.clone(),
        )?,
    );
    store.data_mut().route_overrides = Arc::clone(&execution.route_overrides);
    store.data_mut().host_load = Arc::clone(&execution.host_load);
    store.data_mut().bridge_manager = Arc::clone(&execution.bridge_manager);
    store.limiter(|state| &mut state.limits);
    maybe_set_guest_fuel_budget(&mut store, execution)?;

    if let Ok(bindings) = control_plane_component_bindings::ControlPlaneFaas::instantiate(
        &mut store, component, &linker,
    ) {
        record_wasm_start(execution.telemetry.as_ref());
        let response = bindings.tachyon_mesh_handler().call_handle_request(
            &mut store,
            &control_plane_component_bindings::exports::tachyon::mesh::handler::Request {
                method: request.method,
                uri: request.uri,
                headers: request.headers,
                body: request.body.to_vec(),
                trailers: request.trailers,
            },
        );
        record_wasm_end(execution.telemetry.as_ref());
        let fuel_consumed = sampled_fuel_consumed(&mut store, execution)?;
        let response = response.map_err(|error| {
            guest_execution_error(
                error,
                "control-plane guest component `handle-request` trapped",
            )
        })?;
        let status = StatusCode::from_u16(response.status).map_err(|error| {
            ExecutionError::Internal(format!(
                "control-plane guest component returned an invalid HTTP status code `{}`: {error}",
                response.status
            ))
        })?;

        return Ok(GuestExecutionOutcome {
            output: GuestExecutionOutput::Http(GuestHttpResponse {
                status,
                headers: response.headers,
                body: Bytes::from(response.body),
                trailers: response.trailers,
            }),
            fuel_consumed,
        });
    }

    let bindings =
        system_component_bindings::SystemFaasGuest::instantiate(&mut store, component, &linker)
            .map_err(|error| {
                guest_execution_error(
                    error,
                    format!(
                        "failed to instantiate system guest component from {}",
                        component_path.display()
                    ),
                )
            })?;
    record_wasm_start(execution.telemetry.as_ref());
    let response = bindings.tachyon_mesh_handler().call_handle_request(
        &mut store,
        &system_component_bindings::exports::tachyon::mesh::handler::Request {
            method: request.method,
            uri: request.uri,
            headers: request.headers,
            body: request.body.to_vec(),
            trailers: request.trailers,
        },
    );
    record_wasm_end(execution.telemetry.as_ref());
    let fuel_consumed = sampled_fuel_consumed(&mut store, execution)?;
    let response = response.map_err(|error| {
        guest_execution_error(error, "system guest component `handle-request` trapped")
    })?;
    let status = StatusCode::from_u16(response.status).map_err(|error| {
        ExecutionError::Internal(format!(
            "system guest component returned an invalid HTTP status code `{}`: {error}",
            response.status
        ))
    })?;

    Ok(GuestExecutionOutcome {
        output: GuestExecutionOutput::Http(GuestHttpResponse {
            status,
            headers: response.headers,
            body: Bytes::from(response.body),
            trailers: response.trailers,
        }),
        fuel_consumed,
    })
}

impl BackgroundTickRunner {
    #[allow(clippy::too_many_arguments)]
    fn new(
        engine: &Engine,
        config: &IntegrityConfig,
        route: &IntegrityRoute,
        function_name: &str,
        telemetry: TelemetryHandle,
        concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
        host_identity: Arc<HostIdentity>,
        storage_broker: Arc<StorageBrokerManager>,
        route_overrides: Arc<ArcSwap<HashMap<String, String>>>,
        peer_capabilities: PeerCapabilityCache,
        host_capabilities: Capabilities,
        host_load: Arc<HostLoadCounters>,
    ) -> std::result::Result<Self, ExecutionError> {
        let module_path = resolve_guest_module_path(function_name)
            .map_err(ExecutionError::GuestModuleNotFound)?;
        let component = Component::from_file(engine, &module_path).map_err(|error| {
            guest_execution_error(
                error,
                format!(
                    "failed to load background system component from {}",
                    module_path.display()
                ),
            )
        })?;

        let mut linker = ComponentLinker::new(engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
            guest_execution_error(
                error,
                "failed to add WASI preview2 functions to background component linker",
            )
        })?;
        background_component_bindings::tachyon::mesh::scaling_metrics::add_to_linker::<
            ComponentHostState,
            ComponentHostState,
        >(&mut linker, |state: &mut ComponentHostState| state)
        .map_err(|error| {
            guest_execution_error(
                error,
                "failed to add scaling metrics functions to background component linker",
            )
        })?;
        background_component_bindings::tachyon::mesh::telemetry_reader::add_to_linker::<
            ComponentHostState,
            ComponentHostState,
        >(&mut linker, |state: &mut ComponentHostState| state)
        .map_err(|error| {
            guest_execution_error(
                error,
                "failed to add telemetry reader functions to background component linker",
            )
        })?;
        background_component_bindings::tachyon::mesh::outbound_http::add_to_linker::<
            ComponentHostState,
            ComponentHostState,
        >(&mut linker, |state: &mut ComponentHostState| state)
        .map_err(|error| {
            guest_execution_error(
                error,
                "failed to add outbound HTTP functions to background component linker",
            )
        })?;
        background_component_bindings::tachyon::mesh::outbox_store::add_to_linker::<
            ComponentHostState,
            ComponentHostState,
        >(&mut linker, |state: &mut ComponentHostState| state)
        .map_err(|error| {
            guest_execution_error(
                error,
                "failed to add outbox store functions to background component linker",
            )
        })?;
        background_component_bindings::tachyon::mesh::routing_control::add_to_linker::<
            ComponentHostState,
            ComponentHostState,
        >(&mut linker, |state: &mut ComponentHostState| state)
        .map_err(|error| {
            guest_execution_error(
                error,
                "failed to add routing control functions to background component linker",
            )
        })?;

        let mut store = Store::new(
            engine,
            ComponentHostState::new(
                route,
                config.clone(),
                config.guest_memory_limit_bytes,
                telemetry,
                SecretAccess::default(),
                HeaderMap::new(),
                host_identity,
                storage_broker,
                concurrency_limits,
                Vec::new(),
            )?,
        );
        store.data_mut().route_overrides = route_overrides;
        store.data_mut().peer_capabilities = peer_capabilities;
        store.data_mut().host_capabilities = host_capabilities;
        store.data_mut().host_load = host_load;
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(config.guest_fuel_budget)
            .map_err(|error| guest_execution_error(error, "failed to inject guest fuel budget"))?;

        let bindings = if let Ok(bindings) =
            control_plane_component_bindings::ControlPlaneFaas::instantiate(
                &mut store, &component, &linker,
            ) {
            BackgroundGuestBindings::ControlPlane(bindings)
        } else {
            BackgroundGuestBindings::Background(
                background_component_bindings::BackgroundSystemFaas::instantiate(
                    &mut store, &component, &linker,
                )
                .map_err(|error| {
                    guest_execution_error(
                        error,
                        format!(
                            "failed to instantiate background system component from {}",
                            module_path.display()
                        ),
                    )
                })?,
            )
        };

        Ok(Self {
            function_name: function_name.to_owned(),
            route_path: route.path.clone(),
            store,
            bindings,
        })
    }

    fn tick(&mut self) -> std::result::Result<(), ExecutionError> {
        match &self.bindings {
            BackgroundGuestBindings::Background(bindings) => {
                bindings.call_on_tick(&mut self.store).map_err(|error| {
                    guest_execution_error(error, "background system guest `on-tick` trapped")
                })
            }
            BackgroundGuestBindings::ControlPlane(bindings) => {
                bindings.call_on_tick(&mut self.store).map_err(|error| {
                    guest_execution_error(error, "control-plane system guest `on-tick` trapped")
                })
            }
        }
    }
}

fn execute_legacy_guest(
    engine: &Engine,
    function_name: &str,
    body: Bytes,
    route: &IntegrityRoute,
    module_path: &Path,
    module: Module,
    execution: &GuestExecutionContext,
) -> std::result::Result<GuestExecutionOutcome, ExecutionError> {
    let linker = build_linker(engine)?;
    let stdin_file = create_guest_stdin_file(&body)?;
    let stdout_capture = AsyncGuestOutputCapture::new(
        function_name,
        GuestLogStreamType::Stdout,
        execution.async_log_sender.clone(),
        true,
        execution.config.max_stdout_bytes,
    );
    let stderr_capture = AsyncGuestOutputCapture::new(
        function_name,
        GuestLogStreamType::Stderr,
        execution.async_log_sender.clone(),
        false,
        0,
    );
    let mut wasi = WasiCtxBuilder::new();
    wasi.arg(legacy_guest_program_name(module_path))
        .stdin(InputFile::new(stdin_file.file.try_clone().map_err(
            |error| guest_execution_error(error.into(), "failed to clone guest stdin file handle"),
        )?))
        .stdout(stdout_capture.clone())
        .stderr(stderr_capture);
    let traceparent = trace_context_for_request(&execution.request_headers);
    add_route_environment_with_trace(
        &mut wasi,
        route,
        execution.host_identity.as_ref(),
        Some(&traceparent),
    )?;

    if let Some(module_dir) = module_path.parent() {
        wasi.preopened_dir(module_dir, ".", DirPerms::READ, FilePerms::READ)
            .map_err(|error| {
                guest_execution_error(
                    error,
                    format!(
                        "failed to preopen guest module directory {}",
                        module_dir.display()
                    ),
                )
            })?;
    }

    preopen_route_volumes(&mut wasi, route)?;

    let wasi = wasi.build_p1();
    let mut store = Store::new(
        engine,
        LegacyHostState::new(
            wasi,
            execution.config.guest_memory_limit_bytes,
            #[cfg(feature = "ai-inference")]
            Arc::clone(&execution.ai_runtime),
        ),
    );
    store.limiter(|state| &mut state.limits);
    maybe_set_guest_fuel_budget(&mut store, execution)?;
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|error| guest_execution_error(error, "failed to instantiate guest module"))?;
    let (entrypoint_name, entrypoint) =
        resolve_guest_entrypoint(&mut store, &instance).map_err(|error| {
            guest_execution_error(
                error,
                "failed to resolve exported function `faas_entry` or `_start`",
            )
        })?;

    record_wasm_start(execution.telemetry.as_ref());
    let call_result = entrypoint.call(&mut store, ());
    record_wasm_end(execution.telemetry.as_ref());
    let fuel_consumed = sampled_fuel_consumed(&mut store, execution)?;
    handle_guest_entrypoint_result(entrypoint_name, call_result)?;
    let stdout_bytes = stdout_capture.finish()?;

    Ok(GuestExecutionOutcome {
        output: GuestExecutionOutput::LegacyStdout(stdout_bytes),
        fuel_consumed,
    })
}

fn execute_legacy_guest_with_stdio(
    engine: &Engine,
    route: &IntegrityRoute,
    module_path: &Path,
    module: Module,
    execution: &GuestExecutionContext,
    stdin: impl StdinStream + 'static,
    stdout: impl StdoutStream + 'static,
) -> std::result::Result<(), ExecutionError> {
    let linker = build_linker(engine)?;
    let mut wasi = WasiCtxBuilder::new();
    let traceparent = trace_context_for_request(&execution.request_headers);
    add_route_environment_with_trace(
        &mut wasi,
        route,
        execution.host_identity.as_ref(),
        Some(&traceparent),
    )?;
    wasi.arg(legacy_guest_program_name(module_path))
        .stdin(stdin)
        .stdout(stdout);

    if let Some(module_dir) = module_path.parent() {
        wasi.preopened_dir(module_dir, ".", DirPerms::READ, FilePerms::READ)
            .map_err(|error| {
                guest_execution_error(
                    error,
                    format!(
                        "failed to preopen guest module directory {}",
                        module_dir.display()
                    ),
                )
            })?;
    }

    preopen_route_volumes(&mut wasi, route)?;

    let wasi = wasi.build_p1();
    let mut store = Store::new(
        engine,
        LegacyHostState::new(
            wasi,
            execution.config.guest_memory_limit_bytes,
            #[cfg(feature = "ai-inference")]
            Arc::clone(&execution.ai_runtime),
        ),
    );
    store.limiter(|state| &mut state.limits);
    maybe_set_guest_fuel_budget(&mut store, execution)?;
    let instance = linker
        .instantiate(&mut store, &module)
        .map_err(|error| guest_execution_error(error, "failed to instantiate guest module"))?;
    let (entrypoint_name, entrypoint) =
        resolve_guest_entrypoint(&mut store, &instance).map_err(|error| {
            guest_execution_error(
                error,
                "failed to resolve exported function `faas_entry` or `_start`",
            )
        })?;

    record_wasm_start(execution.telemetry.as_ref());
    let call_result = entrypoint.call(&mut store, ());
    record_wasm_end(execution.telemetry.as_ref());
    let _ = sampled_fuel_consumed(&mut store, execution)?;
    handle_guest_entrypoint_result(entrypoint_name, call_result)?;
    Ok(())
}

#[derive(Clone)]
struct TcpSocketStdin {
    socket: Arc<Mutex<std::net::TcpStream>>,
}

impl TcpSocketStdin {
    fn new(socket: std::net::TcpStream) -> Self {
        Self {
            socket: Arc::new(Mutex::new(socket)),
        }
    }
}

impl IsTerminal for TcpSocketStdin {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdinStream for TcpSocketStdin {
    fn p2_stream(&self) -> Box<dyn InputStream> {
        Box::new(self.clone())
    }

    fn async_stream(&self) -> Box<dyn tokio::io::AsyncRead + Send + Sync> {
        Box::new(tokio::io::empty())
    }
}

#[async_trait::async_trait]
impl InputStream for TcpSocketStdin {
    fn read(&mut self, size: usize) -> StreamResult<Bytes> {
        if size == 0 {
            return Ok(Bytes::new());
        }

        let mut socket = self
            .socket
            .lock()
            .map_err(|_| StreamError::trap("tcp stdin socket lock poisoned"))?;
        let mut buffer = vec![0_u8; size];
        loop {
            match std::io::Read::read(&mut *socket, &mut buffer) {
                Ok(0) => return Err(StreamError::Closed),
                Ok(read) => {
                    buffer.truncate(read);
                    return Ok(Bytes::from(buffer));
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(StreamError::LastOperationFailed(error.into())),
            }
        }
    }
}

#[async_trait::async_trait]
impl Pollable for TcpSocketStdin {
    async fn ready(&mut self) {}
}

#[derive(Clone)]
struct TcpSocketStdout {
    socket: Arc<Mutex<std::net::TcpStream>>,
}

impl TcpSocketStdout {
    fn new(socket: std::net::TcpStream) -> Self {
        Self {
            socket: Arc::new(Mutex::new(socket)),
        }
    }
}

impl IsTerminal for TcpSocketStdout {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for TcpSocketStdout {
    fn p2_stream(&self) -> Box<dyn OutputStream> {
        Box::new(self.clone())
    }

    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        Box::new(tokio::io::sink())
    }
}

#[async_trait::async_trait]
impl OutputStream for TcpSocketStdout {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        let mut socket = self
            .socket
            .lock()
            .map_err(|_| StreamError::trap("tcp stdout socket lock poisoned"))?;
        loop {
            match std::io::Write::write_all(&mut *socket, &bytes) {
                Ok(()) => return Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(StreamError::LastOperationFailed(error.into())),
            }
        }
    }

    fn flush(&mut self) -> StreamResult<()> {
        let mut socket = self
            .socket
            .lock()
            .map_err(|_| StreamError::trap("tcp stdout socket lock poisoned"))?;
        std::io::Write::flush(&mut *socket)
            .map_err(|error| StreamError::LastOperationFailed(error.into()))
    }

    fn check_write(&mut self) -> StreamResult<usize> {
        Ok(4096)
    }
}

#[async_trait::async_trait]
impl Pollable for TcpSocketStdout {
    async fn ready(&mut self) {}
}

#[derive(Default)]
struct AsyncGuestOutputState {
    function_name: String,
    capture_response: bool,
    max_response_bytes: usize,
    response: Vec<u8>,
    pending: Vec<u8>,
    response_overflowed: bool,
    sender: Option<mpsc::Sender<AsyncLogEntry>>,
    stream_type: Option<GuestLogStreamType>,
}

#[derive(Clone, Default)]
struct AsyncGuestOutputCapture {
    state: Arc<Mutex<AsyncGuestOutputState>>,
}

impl AsyncGuestOutputCapture {
    fn new(
        function_name: impl Into<String>,
        stream_type: GuestLogStreamType,
        sender: mpsc::Sender<AsyncLogEntry>,
        capture_response: bool,
        max_response_bytes: usize,
    ) -> Self {
        Self {
            state: Arc::new(Mutex::new(AsyncGuestOutputState {
                function_name: function_name.into(),
                capture_response,
                max_response_bytes,
                response: Vec::new(),
                pending: Vec::new(),
                response_overflowed: false,
                sender: Some(sender),
                stream_type: Some(stream_type),
            })),
        }
    }

    fn finish(&self) -> std::result::Result<Bytes, ExecutionError> {
        let mut state = self.state.lock().map_err(|_| {
            ExecutionError::Internal("guest async stdout capture lock poisoned".to_owned())
        })?;
        flush_async_guest_output(&mut state);
        if state.response_overflowed {
            return Err(ExecutionError::ResourceLimitExceeded {
                kind: ResourceLimitKind::Stdout,
                detail: format!(
                    "guest wrote more than {} response bytes to stdout",
                    state.max_response_bytes
                ),
            });
        }

        Ok(Bytes::from(std::mem::take(&mut state.response)))
    }
}

impl IsTerminal for AsyncGuestOutputCapture {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for AsyncGuestOutputCapture {
    fn p2_stream(&self) -> Box<dyn OutputStream> {
        Box::new(self.clone())
    }

    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        Box::new(tokio::io::sink())
    }
}

#[async_trait::async_trait]
impl OutputStream for AsyncGuestOutputCapture {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StreamError::trap("guest async stdout capture lock poisoned"))?;
        state.pending.extend_from_slice(&bytes);
        while let Some(position) = state.pending.iter().position(|byte| *byte == b'\n') {
            let segment = state.pending.drain(..=position).collect::<Vec<_>>();
            handle_async_guest_segment(&mut state, &segment);
        }
        Ok(())
    }

    fn flush(&mut self) -> StreamResult<()> {
        Ok(())
    }

    fn check_write(&mut self) -> StreamResult<usize> {
        Ok(4096)
    }
}

#[async_trait::async_trait]
impl Pollable for AsyncGuestOutputCapture {
    async fn ready(&mut self) {}
}

fn disconnected_log_sender() -> mpsc::Sender<AsyncLogEntry> {
    let (sender, _receiver) = mpsc::channel(1);
    sender
}

struct GuestTempFile {
    path: PathBuf,
    file: fs::File,
}

impl Drop for GuestTempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn create_guest_stdin_file(body: &Bytes) -> std::result::Result<GuestTempFile, ExecutionError> {
    let path = guest_temp_file_path("stdin");
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .read(true)
        .open(&path)
        .map_err(|error| {
            guest_execution_error(error.into(), "failed to create guest stdin temp file")
        })?;
    file.write_all(body).map_err(|error| {
        guest_execution_error(error.into(), "failed to write guest stdin temp file")
    })?;
    file.flush().map_err(|error| {
        guest_execution_error(error.into(), "failed to flush guest stdin temp file")
    })?;
    file.sync_all().map_err(|error| {
        guest_execution_error(error.into(), "failed to sync guest stdin temp file to disk")
    })?;
    drop(file);
    let file = fs::File::open(&path).map_err(|error| {
        guest_execution_error(error.into(), "failed to reopen guest stdin temp file")
    })?;
    Ok(GuestTempFile { path, file })
}

#[cfg(test)]
fn create_guest_stdout_file() -> std::result::Result<GuestTempFile, ExecutionError> {
    let path = guest_temp_file_path("stdout");
    let file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .read(true)
        .open(&path)
        .map_err(|error| {
            guest_execution_error(error.into(), "failed to create guest stdout temp file")
        })?;
    Ok(GuestTempFile { path, file })
}

fn guest_temp_file_path(kind: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!("tachyon-{kind}-{}.tmp", Uuid::new_v4()));
    path
}

#[cfg(test)]
fn read_guest_stdout_file(
    path: &Path,
    max_stdout_bytes: usize,
) -> std::result::Result<Bytes, ExecutionError> {
    let stdout = fs::read(path).map_err(|error| {
        guest_execution_error(error.into(), "failed to read guest stdout temp file")
    })?;
    if stdout.len() > max_stdout_bytes {
        return Err(ExecutionError::ResourceLimitExceeded {
            kind: ResourceLimitKind::Stdout,
            detail: format!(
                "guest wrote {} bytes to stdout with a configured limit of {} bytes",
                stdout.len(),
                max_stdout_bytes
            ),
        });
    }
    Ok(Bytes::from(stdout))
}

fn maybe_set_guest_fuel_budget<T>(
    store: &mut Store<T>,
    execution: &GuestExecutionContext,
) -> std::result::Result<(), ExecutionError> {
    if !execution.sampled_execution {
        return Ok(());
    }

    store
        .set_fuel(execution.config.guest_fuel_budget)
        .map_err(|error| guest_execution_error(error, "failed to inject guest fuel budget"))
}

fn sampled_fuel_consumed<T>(
    store: &mut Store<T>,
    execution: &GuestExecutionContext,
) -> std::result::Result<Option<u64>, ExecutionError> {
    if !execution.sampled_execution {
        return Ok(None);
    }

    let remaining = store
        .get_fuel()
        .map_err(|error| guest_execution_error(error, "failed to read remaining guest fuel"))?;
    Ok(Some(
        execution.config.guest_fuel_budget.saturating_sub(remaining),
    ))
}

fn legacy_guest_program_name(module_path: &Path) -> String {
    module_path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| format!("./{name}"))
        .unwrap_or_else(|| "./guest.wasm".to_owned())
}

fn resolve_guest_entrypoint(
    store: &mut Store<LegacyHostState>,
    instance: &Instance,
) -> std::result::Result<(&'static str, TypedFunc<(), ()>), wasmtime::Error> {
    match instance.get_typed_func(&mut *store, "faas_entry") {
        Ok(entrypoint) => Ok(("faas_entry", entrypoint)),
        Err(_) => instance
            .get_typed_func(&mut *store, "_start")
            .map(|entrypoint| ("_start", entrypoint)),
    }
}

fn build_linker(
    engine: &Engine,
) -> std::result::Result<ModuleLinker<LegacyHostState>, ExecutionError> {
    let mut linker = ModuleLinker::new(engine);
    p1::add_to_linker_sync(&mut linker, |state: &mut LegacyHostState| &mut state.wasi).map_err(
        |error| guest_execution_error(error, "failed to add WASI preview1 functions to linker"),
    )?;
    #[cfg(feature = "ai-inference")]
    wasmtime_wasi_nn::witx::add_to_linker(&mut linker, |state: &mut LegacyHostState| {
        &mut state.wasi_nn
    })
    .map_err(|error| guest_execution_error(error, "failed to add WASI-NN functions to linker"))?;
    Ok(linker)
}

#[cfg_attr(feature = "ai-inference", allow(dead_code))]
fn requires_ai_inference_feature(function_name: &str) -> bool {
    normalize_target_module_name(function_name) == "guest-ai"
}

fn resolve_function_name(path: &str) -> Option<String> {
    path.split('/')
        .rev()
        .find(|segment| !segment.is_empty() && *segment != "api")
        .map(ToOwned::to_owned)
}

fn default_route_name(path: &str) -> String {
    resolve_function_name(path).unwrap_or_else(|| path.trim_matches('/').to_owned())
}

fn background_route_module(route: &IntegrityRoute) -> Option<String> {
    route
        .targets
        .first()
        .map(|target| target.module.clone())
        .or_else(|| resolve_function_name(&route.path))
}

fn normalize_route_path(path: &str) -> String {
    let trimmed = path.trim();
    let with_leading_slash = if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    };
    let normalized = with_leading_slash.trim_end_matches('/');

    if normalized.is_empty() {
        "/".to_owned()
    } else {
        normalized.to_owned()
    }
}

fn normalize_route_override_key(route_key: &str) -> String {
    if let Some(route_path) = route_key.strip_prefix(MESH_QOS_OVERRIDE_PREFIX) {
        return format!(
            "{MESH_QOS_OVERRIDE_PREFIX}{}",
            normalize_route_path(route_path)
        );
    }

    normalize_route_path(route_key)
}

fn route_path_for_override_key(route_key: &str) -> String {
    route_key
        .strip_prefix(MESH_QOS_OVERRIDE_PREFIX)
        .map(normalize_route_path)
        .unwrap_or_else(|| normalize_route_path(route_key))
}

fn resolve_guest_module_path(
    function_name: &str,
) -> std::result::Result<PathBuf, GuestModuleNotFound> {
    if system_storage::is_asset_uri(function_name) {
        return system_storage::resolve_asset_uri(&integrity_manifest_path(), function_name)
            .map_err(|error| GuestModuleNotFound::new(function_name, error.to_string()));
    }

    let candidates = guest_module_candidate_paths(function_name);
    let candidate_strings = candidates
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();

    candidates
        .into_iter()
        .find(|path| path.exists())
        .map(normalize_path)
        .ok_or_else(|| {
            GuestModuleNotFound::new(function_name, format_candidate_list(&candidate_strings))
        })
}

fn guest_module_candidate_paths(function_name: &str) -> Vec<PathBuf> {
    let wasm_file = format!(
        "{}.wasm",
        normalize_target_module_name(function_name).replace('-', "_")
    );
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest_relative_candidates = [
        format!("../target/wasm32-wasip2/debug/{wasm_file}"),
        format!("../target/wasm32-wasip2/release/{wasm_file}"),
        format!("../target/wasm32-wasip1/debug/{wasm_file}"),
        format!("../target/wasm32-wasip1/release/{wasm_file}"),
        format!("../target/wasm32-wasi/debug/{wasm_file}"),
        format!("../target/wasm32-wasi/release/{wasm_file}"),
    ];
    let workspace_relative_candidates = [
        format!("target/wasm32-wasip2/debug/{wasm_file}"),
        format!("target/wasm32-wasip2/release/{wasm_file}"),
        format!("target/wasm32-wasip1/debug/{wasm_file}"),
        format!("target/wasm32-wasip1/release/{wasm_file}"),
        format!("target/wasm32-wasi/debug/{wasm_file}"),
        format!("target/wasm32-wasi/release/{wasm_file}"),
        format!("guest-modules/{wasm_file}"),
    ];

    manifest_relative_candidates
        .into_iter()
        .map(|path| manifest_dir.join(path))
        .chain(workspace_relative_candidates.into_iter().map(PathBuf::from))
        .chain(std::iter::once(PathBuf::from(format!(
            "/app/guest-modules/{wasm_file}"
        ))))
        .collect()
}

fn normalize_target_module_name(module_name: &str) -> String {
    module_name
        .trim()
        .strip_suffix(".wasm")
        .unwrap_or(module_name.trim())
        .trim()
        .to_owned()
}

fn normalize_path(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

fn format_candidate_list(paths: &[String]) -> String {
    paths.join(", ")
}

fn record_wasm_start(telemetry: Option<&GuestTelemetryContext>) {
    record_wasm_event(telemetry, true);
}

fn record_wasm_end(telemetry: Option<&GuestTelemetryContext>) {
    record_wasm_event(telemetry, false);
}

fn record_wasm_event(telemetry: Option<&GuestTelemetryContext>, is_start: bool) {
    let Some(telemetry) = telemetry else {
        return;
    };

    let event = if is_start {
        TelemetryEvent::WasmStart {
            trace_id: telemetry.trace_id.clone(),
            timestamp: Instant::now(),
        }
    } else {
        TelemetryEvent::WasmEnd {
            trace_id: telemetry.trace_id.clone(),
            timestamp: Instant::now(),
        }
    };

    telemetry::record_event(&telemetry.handle, event);
}

fn should_shed_system_route(telemetry: &TelemetryHandle) -> bool {
    is_system_route_saturated(telemetry::active_requests(telemetry))
}

fn is_system_route_saturated(active_requests: usize) -> bool {
    active_requests > SYSTEM_ROUTE_ACTIVE_REQUEST_THRESHOLD
}

fn handle_guest_entrypoint_result(
    entrypoint_name: &str,
    result: std::result::Result<(), wasmtime::Error>,
) -> std::result::Result<(), ExecutionError> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if entrypoint_name == "_start" => match error.downcast_ref::<I32Exit>() {
            Some(exit) => {
                if exit.0 != 0 {
                    tracing::warn!(
                        guest_entrypoint = entrypoint_name,
                        exit_status = exit.0,
                        "command-style WASI guest exited non-zero; preserving stdout response"
                    );
                }
                Ok(())
            }
            None => Err(guest_execution_error(
                error,
                format!("guest function `{entrypoint_name}` trapped"),
            )),
        },
        Err(error) => Err(guest_execution_error(
            error,
            format!("guest function `{entrypoint_name}` trapped"),
        )),
    }
}

fn build_concurrency_limits(
    config: &IntegrityConfig,
) -> Arc<HashMap<String, Arc<RouteExecutionControl>>> {
    Arc::new(
        config
            .routes
            .iter()
            .map(|route| {
                (
                    route.path.clone(),
                    Arc::new(RouteExecutionControl::new(route)),
                )
            })
            .collect(),
    )
}

fn total_route_concurrency(routes: &[IntegrityRoute]) -> Result<u32> {
    u32::try_from(
        routes
            .iter()
            .map(|route| u64::from(route.max_concurrency))
            .sum::<u64>(),
    )
    .context("embedded sealed configuration declares more route concurrency than Wasmtime can pool")
}

fn total_min_instances(routes: &[IntegrityRoute]) -> Result<u32> {
    u32::try_from(
        routes
            .iter()
            .map(|route| u64::from(route.min_instances))
            .sum::<u64>(),
    )
    .context("embedded sealed configuration declares more warm instances than Wasmtime can track")
}

impl RouteExecutionControl {
    fn new(route: &IntegrityRoute) -> Self {
        Self::from_limits(route.min_instances, route.max_concurrency)
    }

    fn from_limits(min_instances: u32, max_concurrency: u32) -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(
                usize::try_from(max_concurrency)
                    .expect("route max_concurrency should fit in usize"),
            )),
            pending_waiters: AtomicUsize::new(0),
            active_requests: AtomicUsize::new(0),
            draining: AtomicBool::new(false),
            draining_since: Mutex::new(None),
            min_instances,
            max_concurrency,
            prewarmed_instances: AtomicUsize::new(0),
        }
    }

    fn pending_queue_size(&self) -> u32 {
        self.pending_waiters
            .load(Ordering::Relaxed)
            .min(u32::MAX as usize) as u32
    }

    fn keda_pending_queue_size(&self) -> u32 {
        let pending = self.pending_queue_size();
        if pending == 0 {
            return 0;
        }

        let active = self.active_requests.load(Ordering::Relaxed);
        if active < self.max_concurrency as usize {
            return pending;
        }

        pending.saturating_add(self.max_concurrency)
    }

    fn record_prewarm_success(&self) {
        self.prewarmed_instances.fetch_add(1, Ordering::SeqCst);
    }

    fn begin_request(self: &Arc<Self>) -> ActiveRouteRequestGuard {
        ActiveRouteRequestGuard::new(Arc::clone(self))
    }

    fn mark_draining(&self, started_at: Instant) {
        self.draining.store(true, Ordering::SeqCst);
        *self
            .draining_since
            .lock()
            .expect("route lifecycle state should not be poisoned") = Some(started_at);
    }

    fn force_terminate(&self) {
        self.semaphore.close();
    }

    fn lifecycle_state(&self) -> RouteLifecycleState {
        if self.draining.load(Ordering::SeqCst) {
            RouteLifecycleState::Draining
        } else {
            RouteLifecycleState::Active
        }
    }

    fn active_request_count(&self) -> usize {
        self.active_requests.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    fn prewarmed_instances(&self) -> u32 {
        self.prewarmed_instances
            .load(Ordering::SeqCst)
            .min(u32::MAX as usize) as u32
    }
}
