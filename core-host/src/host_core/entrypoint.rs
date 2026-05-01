use super::*;

pub(crate) async fn run() -> Result<()> {
    init_host_tracing();
    ensure_rustls_crypto_provider();
    let cli = HostCli::parse();
    match cli.command.unwrap_or(HostCommand::Serve) {
        HostCommand::Serve => serve_host(cli.accel).await,
        HostCommand::Run(command) => {
            let exit_code = match execute_batch_target_from_manifest(
                command.manifest.unwrap_or_else(integrity_manifest_path),
                &command.target,
            )
            .await
            {
                Ok(true) => 0,
                Ok(false) => 1,
                Err(error) => {
                    tracing::error!("batch target `{}` failed: {error:#}", command.target);
                    1
                }
            };
            std::process::exit(exit_code);
        }
    }
}

pub(crate) async fn serve_host(accel: AccelerationMode) -> Result<()> {
    let manifest_path = integrity_manifest_path();
    let (export_sender, export_receiver) = mpsc::channel(TELEMETRY_EXPORT_QUEUE_CAPACITY);
    let telemetry =
        telemetry::init_telemetry_with_emitter(move |line| export_sender.try_send(line).is_ok());
    let memory_governor = Arc::new(memory_governor::MemoryGovernor::from_system_memory());
    let runtime = build_runtime_state(verify_integrity()?)?;
    maybe_init_l4_acceleration(accel, &runtime.config.layer4);
    if maybe_run_bootstrap_mode(&runtime.config).await? {
        return Ok(());
    }
    let core_store = open_core_store_for_manifest(&manifest_path).await?;
    secure_cache_bootstrap(core_store.as_ref(), &runtime)?;
    let host_identity = Arc::new(HostIdentity::generate());
    let uds_fast_path = Arc::new(new_uds_fast_path_registry());
    let storage_broker = Arc::new(StorageBrokerManager::new(Arc::clone(&core_store)));
    let bridge_manager = Arc::new(BridgeManager::default());
    let buffered_requests = Arc::new(BufferedRequestManager::new(buffered_request_spool_dir(
        &manifest_path,
    ))?);
    let background_workers = Arc::new(BackgroundWorkerManager::default());
    let route_overrides = Arc::new(ArcSwap::from_pointee(HashMap::new()));
    let peer_capabilities = Arc::new(Mutex::new(HashMap::new()));
    let host_capabilities = Capabilities::detect();
    let host_load = Arc::new(HostLoadCounters::default());
    let tls_manager = Arc::new(tls_runtime::TlsManager::default());
    let mtls_gateway = tls_runtime::load_mtls_gateway_config_from_env()?;
    let auth_manager = Arc::new(auth::AuthManager::new(&manifest_path)?);
    let (async_log_sender, async_log_receiver) = mpsc::channel(LOG_QUEUE_CAPACITY);
    background_workers.start_for_runtime(
        &runtime,
        telemetry.clone(),
        Arc::clone(&host_identity),
        Arc::clone(&storage_broker),
        Arc::clone(&route_overrides),
        Arc::clone(&peer_capabilities),
        host_capabilities,
        Arc::clone(&host_load),
    );

    let state = AppState {
        runtime: Arc::new(ArcSwap::from_pointee(runtime.clone())),
        draining_runtimes: Arc::new(Mutex::new(Vec::new())),
        http_client: Client::new(),
        async_log_sender,
        secrets_vault: SecretsVault::load(),
        host_identity,
        uds_fast_path: Arc::clone(&uds_fast_path),
        storage_broker,
        bridge_manager,
        core_store,
        buffered_requests,
        volume_manager: Arc::new(VolumeManager::default()),
        route_overrides,
        peer_capabilities,
        host_capabilities,
        host_load,
        memory_governor,
        telemetry,
        tls_manager,
        mtls_gateway: mtls_gateway.map(Arc::new),
        auth_manager,
        enrollment_manager: Arc::new(node_enrollment::EnrollmentManager::new()),
        manifest_path,
        background_workers: Arc::clone(&background_workers),
    };
    state.tls_manager.prime_from_store(&state).await?;
    prewarm_runtime_routes(
        &runtime,
        state.telemetry.clone(),
        Arc::clone(&state.host_identity),
        Arc::clone(&state.storage_broker),
    )?;
    spawn_metering_exporter(state.clone(), export_receiver);
    spawn_async_log_exporter(state.clone(), async_log_receiver);
    spawn_reload_watcher(state.clone());
    spawn_manifest_file_watcher(state.clone());
    spawn_authz_purge_subscriber(state.clone());
    spawn_draining_runtime_reaper(state.clone());
    spawn_volume_gc_sweeper(state.clone());
    spawn_buffered_request_replayer(state.clone());
    spawn_global_memory_governor(state.clone());
    spawn_pressure_monitor(state.clone());
    let app = build_app(state.clone());
    let https_listener = start_https_listener(state.clone(), app.clone()).await?;
    let mtls_listener = start_mtls_gateway_listener(state.clone()).await?;
    let http3_listener = start_http3_listener(state.clone(), app.clone()).await?;
    let udp_layer4_listeners = start_udp_layer4_listeners(state.clone()).await?;
    let tcp_layer4_listeners = start_tcp_layer4_listeners(state).await?;
    let uds_server = start_uds_fast_path_listener(app.clone(), &runtime.config, uds_fast_path)?;

    let listener = tokio::net::TcpListener::bind(&runtime.config.host_address)
        .await
        .with_context(|| {
            format!(
                "failed to bind HTTP listener on {}",
                runtime.config.host_address.as_str()
            )
        })?;

    tokio::select! {
        result = serve_http_listener(listener, app.clone()) => {
            result.context("HTTP server exited unexpectedly")?;
        }
        _ = shutdown_signal() => {}
    }

    if let Some(server) = uds_server {
        server.abort();
        let _ = server.await;
    }
    if let Some(listener) = https_listener {
        listener.join_handle.abort();
        let _ = listener.join_handle.await;
    }
    if let Some(listener) = mtls_listener {
        listener.join_handle.abort();
        let _ = listener.join_handle.await;
    }
    if let Some(listener) = http3_listener {
        listener.join_handle.abort();
        let _ = listener.join_handle.await;
    }
    for listener in udp_layer4_listeners {
        for handle in listener.join_handles {
            handle.abort();
            let _ = handle.await;
        }
    }
    for listener in tcp_layer4_listeners {
        listener.join_handle.abort();
        let _ = listener.join_handle.await;
    }
    background_workers.stop_all().await;
    Ok(())
}

pub(crate) fn maybe_init_l4_acceleration(accel: AccelerationMode, layer4: &IntegrityLayer4Config) {
    if accel != AccelerationMode::Ebpf {
        return;
    }

    let route_count = layer4.tcp.len().saturating_add(layer4.udp.len());
    match network::ebpf::init_ebpf_fastpath(route_count) {
        Ok(network::ebpf::EbpfFastPathStatus::Loaded) => {
            tracing::info!("eBPF L4 fast-path loader initialized");
        }
        Ok(network::ebpf::EbpfFastPathStatus::NoRules) => {
            tracing::info!("eBPF L4 fast-path requested, but no L4 routes are configured");
        }
        Ok(network::ebpf::EbpfFastPathStatus::Unsupported) => {
            tracing::warn!(
                "eBPF L4 fast-path is unsupported on this platform; using userspace routing"
            );
        }
        Err(error) => {
            tracing::warn!("{error}");
        }
    }
}

pub(crate) async fn execute_batch_target_from_manifest(
    manifest_path: PathBuf,
    target_name: &str,
) -> Result<bool> {
    let config = load_integrity_config_from_manifest_path(&manifest_path)?;
    let target = BatchTargetRegistry::build(&config)?
        .get(target_name)
        .cloned()
        .ok_or_else(|| anyhow!("sealed manifest does not define batch target `{target_name}`"))?;
    let engine = build_command_engine(&config)?;
    let module_path =
        resolve_guest_module_path(&target.module).map_err(|error| anyhow!(error.to_string()))?;
    let component = Component::from_file(&engine, &module_path).map_err(|error| {
        anyhow!(
            "failed to load batch target component `{}` from {}: {error}",
            target.name,
            module_path.display()
        )
    })?;

    let mut linker = ComponentLinker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(|error| {
        anyhow!("failed to add WASI preview2 functions to batch target linker: {error}")
    })?;

    let mut wasi = WasiCtxBuilder::new();
    let argv0 = module_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(target.module.as_str())
        .to_owned();
    let args = [argv0.as_str()];
    wasi.inherit_stdio().args(&args);
    for (key, value) in &target.env {
        wasi.env(key, value);
    }
    preopen_batch_target_volumes(&mut wasi, &target)?;

    let mut store = Store::new(
        &engine,
        BatchCommandState {
            ctx: wasi.build(),
            table: ResourceTable::new(),
        },
    );
    let command =
        wasmtime_wasi::p2::bindings::Command::instantiate_async(&mut store, &component, &linker)
            .await
            .map_err(|error| {
                anyhow!(
                    "failed to instantiate batch target `{}` from {}: {error}",
                    target.name,
                    module_path.display()
                )
            })?;

    let run_result: std::result::Result<(), ()> = command
        .wasi_cli_run()
        .call_run(&mut store)
        .await
        .map_err(|error| anyhow!("failed to execute batch target `{}`: {error}", target.name))?;
    Ok(run_result.is_ok())
}
