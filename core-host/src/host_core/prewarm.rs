use super::*;

/// Maximum number of distinct `Arc<Module>` entries the in-memory instance pool
/// keeps warm in a single runtime generation. Sized well above any reasonable
pub(crate) fn prewarm_runtime_routes(
    runtime: &RuntimeState,
    telemetry: TelemetryHandle,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
) -> Result<()> {
    let mut warmed_routes = 0_u32;
    let mut warmed_instances = 0_u32;

    for route in &runtime.config.routes {
        let Some(control) = runtime.concurrency_limits.get(&route.path) else {
            continue;
        };
        if control.min_instances == 0 {
            continue;
        }

        let modules = route_modules_for_prewarm(route);
        for function_name in modules {
            for _ in 0..control.min_instances {
                prewarm_route_instance(
                    runtime,
                    route,
                    &function_name,
                    telemetry.clone(),
                    Arc::clone(&host_identity),
                    Arc::clone(&storage_broker),
                )?;
                control.record_prewarm_success();
                warmed_instances = warmed_instances.saturating_add(1);
            }
        }

        tracing::info!(
            route = %route.path,
            min_instances = control.min_instances,
            max_concurrency = control.max_concurrency,
            "prewarmed route capacity"
        );
        warmed_routes = warmed_routes.saturating_add(1);
    }

    if warmed_instances > 0 {
        tracing::info!(
            routes = warmed_routes,
            instances = warmed_instances,
            "completed instance pool prewarming"
        );
    }

    Ok(())
}

pub(crate) fn route_modules_for_prewarm(route: &IntegrityRoute) -> Vec<String> {
    let mut modules = BTreeSet::new();

    if route.targets.is_empty() {
        modules.insert(default_route_name(&route.path));
    } else {
        for target in &route.targets {
            modules.insert(target.module.clone());
        }
    }

    modules.into_iter().collect()
}

#[cfg(feature = "ai-inference")]
pub(crate) fn add_accelerator_interfaces_to_component_linker(
    linker: &mut ComponentLinker<ComponentHostState>,
    ai_runtime: &ai_inference::AiInferenceRuntime,
    context: &str,
) -> std::result::Result<(), ExecutionError> {
    accelerator_component_bindings::tachyon::accelerator::cpu::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            format!("failed to add CPU accelerator functions to {context}"),
        )
    })?;

    if ai_runtime.supports_accelerator(ai_inference::AcceleratorKind::Gpu) {
        accelerator_component_bindings::tachyon::accelerator::gpu::add_to_linker::<
            ComponentHostState,
            ComponentHostState,
        >(linker, |state: &mut ComponentHostState| state)
        .map_err(|error| {
            guest_execution_error(
                error,
                format!("failed to add GPU accelerator functions to {context}"),
            )
        })?;
    }
    if ai_runtime.supports_accelerator(ai_inference::AcceleratorKind::Npu) {
        accelerator_component_bindings::tachyon::accelerator::npu::add_to_linker::<
            ComponentHostState,
            ComponentHostState,
        >(linker, |state: &mut ComponentHostState| state)
        .map_err(|error| {
            guest_execution_error(
                error,
                format!("failed to add NPU accelerator functions to {context}"),
            )
        })?;
    }
    if ai_runtime.supports_accelerator(ai_inference::AcceleratorKind::Tpu) {
        accelerator_component_bindings::tachyon::accelerator::tpu::add_to_linker::<
            ComponentHostState,
            ComponentHostState,
        >(linker, |state: &mut ComponentHostState| state)
        .map_err(|error| {
            guest_execution_error(
                error,
                format!("failed to add TPU accelerator functions to {context}"),
            )
        })?;
    }

    Ok(())
}

pub(crate) fn prewarm_route_instance(
    runtime: &RuntimeState,
    route: &IntegrityRoute,
    function_name: &str,
    telemetry: TelemetryHandle,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
) -> Result<()> {
    let module_path =
        resolve_guest_module_path(function_name).map_err(ExecutionError::GuestModuleNotFound)?;

    if let Ok(component) = Component::from_file(&runtime.engine, &module_path) {
        match prewarm_component_route(
            runtime,
            route,
            function_name,
            &module_path,
            &component,
            telemetry.clone(),
            Arc::clone(&host_identity),
            Arc::clone(&storage_broker),
        ) {
            Ok(()) => return Ok(()),
            Err(error) if should_fall_back_from_component_prewarm(&error) => {}
            Err(error) => {
                return Err(anyhow!(
                    "failed to prewarm component `{}` for route `{}`: {}",
                    module_path.display(),
                    route.path,
                    execution_error_text(&error)
                ));
            }
        }
    }

    prewarm_legacy_route(runtime, route, function_name, &module_path, host_identity).map_err(
        |error| {
            anyhow!(
                "failed to prewarm guest `{function_name}`: {}",
                execution_error_text(&error)
            )
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prewarm_component_route(
    runtime: &RuntimeState,
    route: &IntegrityRoute,
    function_name: &str,
    module_path: &Path,
    component: &Component,
    telemetry: TelemetryHandle,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
) -> std::result::Result<(), ExecutionError> {
    if route.role == RouteRole::System {
        match BackgroundTickRunner::new(
            &runtime.engine,
            &runtime.config,
            route,
            function_name,
            telemetry.clone(),
            Arc::clone(&runtime.concurrency_limits),
            Arc::clone(&host_identity),
            Arc::clone(&storage_broker),
            Arc::new(ArcSwap::from_pointee(HashMap::new())),
            Arc::new(Mutex::new(HashMap::new())),
            Capabilities::detect(),
            Arc::new(HostLoadCounters::default()),
        ) {
            Ok(_) => return Ok(()),
            Err(error) if !should_fall_back_from_component_prewarm(&error) => return Err(error),
            Err(_) => {}
        }

        match prewarm_system_component_instance(
            &runtime.engine,
            runtime.config.clone(),
            runtime.config.guest_memory_limit_bytes,
            route,
            module_path,
            component,
            telemetry,
            host_identity,
            storage_broker,
            Arc::clone(&runtime.concurrency_limits),
        ) {
            Ok(()) => return Ok(()),
            Err(error) if !should_fall_back_from_component_prewarm(&error) => return Err(error),
            Err(_) => {}
        }
    } else {
        if route_has_udp_binding(&runtime.config, function_name) {
            match prewarm_udp_component_instance(
                &runtime.engine,
                runtime.config.clone(),
                runtime.config.guest_memory_limit_bytes,
                route,
                module_path,
                component,
                telemetry.clone(),
                host_identity.clone(),
                storage_broker.clone(),
                Arc::clone(&runtime.concurrency_limits),
            ) {
                Ok(()) => return Ok(()),
                Err(error) if !should_fall_back_from_component_prewarm(&error) => {
                    return Err(error);
                }
                Err(_) => {}
            }
        }

        #[cfg(feature = "websockets")]
        if route
            .targets
            .iter()
            .any(|target| target.websocket && target.module == function_name)
        {
            match prewarm_websocket_component_instance(
                &runtime.engine,
                runtime.config.clone(),
                runtime.config.guest_memory_limit_bytes,
                route,
                module_path,
                component,
                telemetry.clone(),
                host_identity.clone(),
                storage_broker.clone(),
                Arc::clone(&runtime.concurrency_limits),
            ) {
                Ok(()) => return Ok(()),
                Err(error) if !should_fall_back_from_component_prewarm(&error) => {
                    return Err(error);
                }
                Err(_) => {}
            }
        }

        match prewarm_http_component_instance(
            &runtime.engine,
            runtime.config.clone(),
            runtime.config.guest_memory_limit_bytes,
            route,
            module_path,
            component,
            telemetry,
            host_identity,
            storage_broker,
            Arc::clone(&runtime.concurrency_limits),
            #[cfg(feature = "ai-inference")]
            Arc::clone(&runtime.ai_runtime),
        ) {
            Ok(()) => return Ok(()),
            Err(error) if !should_fall_back_from_component_prewarm(&error) => return Err(error),
            Err(_) => {}
        }
    }

    Err(ExecutionError::Internal(format!(
        "component `{}` did not match a supported prewarm world",
        module_path.display()
    )))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prewarm_http_component_instance(
    engine: &Engine,
    runtime_config: IntegrityConfig,
    max_memory_bytes: usize,
    route: &IntegrityRoute,
    module_path: &Path,
    component: &Component,
    telemetry: TelemetryHandle,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
    #[cfg(feature = "ai-inference")] ai_runtime: Arc<ai_inference::AiInferenceRuntime>,
) -> std::result::Result<(), ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to prewarm HTTP component linker",
        )
    })?;
    component_bindings::tachyon::mesh::secrets_vault::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add secrets vault functions to prewarm HTTP component linker",
        )
    })?;
    component_bindings::tachyon::mesh::bridge_controller::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add bridge controller functions to prewarm HTTP component linker",
        )
    })?;
    component_bindings::tachyon::mesh::vector::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add vector store functions to prewarm HTTP component linker",
        )
    })?;
    component_bindings::tachyon::mesh::training::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add training functions to prewarm HTTP component linker",
        )
    })?;
    #[cfg(feature = "ai-inference")]
    add_accelerator_interfaces_to_component_linker(
        &mut linker,
        ai_runtime.as_ref(),
        "prewarm HTTP component linker",
    )?;

    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            runtime_config,
            max_memory_bytes,
            telemetry,
            SecretAccess::default(),
            HeaderMap::new(),
            host_identity,
            storage_broker,
            concurrency_limits,
            Vec::new(),
        )?,
    );
    #[cfg(feature = "ai-inference")]
    {
        store.data_mut().ai_runtime = Some(ai_runtime);
    }
    store.limiter(|state| &mut state.limits);
    let _ = component_bindings::FaasGuest::instantiate(&mut store, component, &linker).map_err(
        |error| {
            guest_execution_error(
                error,
                format!(
                    "failed to instantiate guest component during prewarm from {}",
                    module_path.display()
                ),
            )
        },
    )?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prewarm_udp_component_instance(
    engine: &Engine,
    runtime_config: IntegrityConfig,
    max_memory_bytes: usize,
    route: &IntegrityRoute,
    module_path: &Path,
    component: &Component,
    telemetry: TelemetryHandle,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
) -> std::result::Result<(), ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to prewarm UDP component linker",
        )
    })?;
    udp_component_bindings::tachyon::mesh::secrets_vault::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add secrets vault functions to prewarm UDP component linker",
        )
    })?;

    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            runtime_config,
            max_memory_bytes,
            telemetry,
            SecretAccess::default(),
            HeaderMap::new(),
            host_identity,
            storage_broker,
            concurrency_limits,
            Vec::new(),
        )?,
    );
    store.limiter(|state| &mut state.limits);
    let _ = udp_component_bindings::UdpFaasGuest::instantiate(&mut store, component, &linker)
        .map_err(|error| {
            guest_execution_error(
                error,
                format!(
                    "failed to instantiate UDP guest component during prewarm from {}",
                    module_path.display()
                ),
            )
        })?;
    Ok(())
}

#[cfg(feature = "websockets")]
#[allow(clippy::too_many_arguments)]
pub(crate) fn prewarm_websocket_component_instance(
    engine: &Engine,
    runtime_config: IntegrityConfig,
    max_memory_bytes: usize,
    route: &IntegrityRoute,
    module_path: &Path,
    component: &Component,
    telemetry: TelemetryHandle,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
) -> std::result::Result<(), ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to prewarm WebSocket component linker",
        )
    })?;
    websocket_component_bindings::tachyon::mesh::secrets_vault::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add secrets vault functions to prewarm WebSocket component linker",
        )
    })?;
    websocket_component_bindings::tachyon::mesh::websocket::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WebSocket functions to prewarm component linker",
        )
    })?;

    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            runtime_config,
            max_memory_bytes,
            telemetry,
            SecretAccess::default(),
            HeaderMap::new(),
            host_identity,
            storage_broker,
            concurrency_limits,
            Vec::new(),
        )?,
    );
    store.limiter(|state| &mut state.limits);
    let _ = websocket_component_bindings::WebsocketFaasGuest::instantiate(
        &mut store, component, &linker,
    )
    .map_err(|error| {
        guest_execution_error(
            error,
            format!(
                "failed to instantiate WebSocket guest component during prewarm from {}",
                module_path.display()
            ),
        )
    })?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prewarm_system_component_instance(
    engine: &Engine,
    runtime_config: IntegrityConfig,
    max_memory_bytes: usize,
    route: &IntegrityRoute,
    module_path: &Path,
    component: &Component,
    telemetry: TelemetryHandle,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
) -> std::result::Result<(), ExecutionError> {
    let mut linker = ComponentLinker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker).map_err(|error| {
        guest_execution_error(
            error,
            "failed to add WASI preview2 functions to prewarm system component linker",
        )
    })?;
    control_plane_component_bindings::tachyon::mesh::telemetry_reader::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add telemetry reader functions to prewarm system component linker",
        )
    })?;
    control_plane_component_bindings::tachyon::mesh::scaling_metrics::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add scaling metrics functions to prewarm system component linker",
        )
    })?;
    control_plane_component_bindings::tachyon::mesh::outbound_http::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add outbound HTTP functions to prewarm system component linker",
        )
    })?;
    control_plane_component_bindings::tachyon::mesh::routing_control::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add routing control functions to prewarm system component linker",
        )
    })?;
    system_component_bindings::tachyon::mesh::bridge_controller::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add bridge controller functions to prewarm system component linker",
        )
    })?;
    system_component_bindings::tachyon::mesh::storage_broker::add_to_linker::<
        ComponentHostState,
        ComponentHostState,
    >(&mut linker, |state: &mut ComponentHostState| state)
    .map_err(|error| {
        guest_execution_error(
            error,
            "failed to add storage broker functions to prewarm system component linker",
        )
    })?;

    let mut store = Store::new(
        engine,
        ComponentHostState::new(
            route,
            runtime_config,
            max_memory_bytes,
            telemetry,
            SecretAccess::default(),
            HeaderMap::new(),
            host_identity,
            storage_broker,
            concurrency_limits,
            Vec::new(),
        )?,
    );
    store.limiter(|state| &mut state.limits);
    if control_plane_component_bindings::ControlPlaneFaas::instantiate(
        &mut store, component, &linker,
    )
    .is_ok()
    {
        return Ok(());
    }
    let _ = system_component_bindings::SystemFaasGuest::instantiate(&mut store, component, &linker)
        .map_err(|error| {
            guest_execution_error(
                error,
                format!(
                    "failed to instantiate system guest component during prewarm from {}",
                    module_path.display()
                ),
            )
        })?;
    Ok(())
}

pub(crate) fn prewarm_legacy_route(
    runtime: &RuntimeState,
    route: &IntegrityRoute,
    function_name: &str,
    module_path: &Path,
    host_identity: Arc<HostIdentity>,
) -> std::result::Result<(), ExecutionError> {
    let module = Module::from_file(&runtime.engine, module_path).map_err(|error| {
        guest_execution_error(
            error,
            format!(
                "failed to load legacy guest artifact during prewarm from {}",
                module_path.display()
            ),
        )
    })?;
    let linker = build_linker(&runtime.engine)?;
    let stdin_file = create_guest_stdin_file(&Bytes::new())?;
    let stdout_capture = AsyncGuestOutputCapture::new(
        function_name,
        GuestLogStreamType::Stdout,
        disconnected_log_sender(),
        false,
        runtime.config.max_stdout_bytes,
    );
    let stderr_capture = AsyncGuestOutputCapture::new(
        function_name,
        GuestLogStreamType::Stderr,
        disconnected_log_sender(),
        false,
        0,
    );

    let mut wasi = WasiCtxBuilder::new();
    wasi.arg(legacy_guest_program_name(module_path))
        .stdin(InputFile::new(stdin_file.file.try_clone().map_err(
            |error| {
                guest_execution_error(
                    error.into(),
                    "failed to clone prewarm guest stdin file handle",
                )
            },
        )?))
        .stdout(stdout_capture)
        .stderr(stderr_capture);
    add_route_environment(&mut wasi, route, host_identity.as_ref())?;

    if let Some(module_dir) = module_path.parent() {
        wasi.preopened_dir(module_dir, ".", DirPerms::READ, FilePerms::READ)
            .map_err(|error| {
                guest_execution_error(
                    error,
                    format!(
                        "failed to preopen guest module directory {} during prewarm",
                        module_dir.display()
                    ),
                )
            })?;
    }

    preopen_route_volumes(&mut wasi, route)?;

    let wasi = wasi.build_p1();
    let mut store = Store::new(
        &runtime.engine,
        LegacyHostState::new(
            wasi,
            runtime.config.guest_memory_limit_bytes,
            #[cfg(feature = "ai-inference")]
            Arc::clone(&runtime.ai_runtime),
        ),
    );
    store.limiter(|state| &mut state.limits);
    let _instance = linker.instantiate(&mut store, &module).map_err(|error| {
        guest_execution_error(error, "failed to instantiate legacy guest during prewarm")
    })?;
    Ok(())
}

pub(crate) fn route_has_udp_binding(config: &IntegrityConfig, function_name: &str) -> bool {
    let normalized = normalize_target_module_name(function_name);
    config
        .layer4
        .udp
        .iter()
        .any(|binding| normalize_target_module_name(&binding.target) == normalized)
}

pub(crate) fn should_fall_back_from_component_prewarm(error: &ExecutionError) -> bool {
    match error {
        ExecutionError::Internal(message) => {
            message.contains("no exported instance named")
                || message.contains("does not export")
                || message.contains("on-connect")
                || message.contains("handle-packet")
        }
        _ => false,
    }
}

pub(crate) fn execution_error_text(error: &ExecutionError) -> String {
    match error {
        ExecutionError::GuestModuleNotFound(details) => details.to_string(),
        ExecutionError::ResourceLimitExceeded { detail, .. } => detail.clone(),
        ExecutionError::Internal(message) => message.clone(),
    }
}
