impl BackgroundWorkerManager {
    #[allow(clippy::too_many_arguments)]
    fn start_for_runtime(
        &self,
        runtime: &RuntimeState,
        telemetry: TelemetryHandle,
        host_identity: Arc<HostIdentity>,
        storage_broker: Arc<StorageBrokerManager>,
        route_overrides: Arc<ArcSwap<HashMap<String, String>>>,
        peer_capabilities: PeerCapabilityCache,
        host_capabilities: Capabilities,
        host_load: Arc<HostLoadCounters>,
    ) {
        let mut new_workers = Vec::new();
        let mut started_workers = 0_u32;

        for route in &runtime.config.routes {
            if route.role != RouteRole::System {
                continue;
            }

            let Some(function_name) = background_route_module(route) else {
                continue;
            };

            if resolve_guest_module_path(&function_name).is_err() {
                tracing::warn!(
                    route = %route.path,
                    function = function_name,
                    "sealed system route is missing its guest artifact"
                );
                continue;
            }

            if BackgroundTickRunner::new(
                &runtime.metered_engine,
                &runtime.config,
                route,
                &function_name,
                telemetry.clone(),
                Arc::clone(&runtime.concurrency_limits),
                Arc::clone(&host_identity),
                Arc::clone(&storage_broker),
                Arc::clone(&route_overrides),
                Arc::clone(&peer_capabilities),
                host_capabilities,
                Arc::clone(&host_load),
            )
            .is_err()
            {
                continue;
            }

            let stop_requested = Arc::new(AtomicBool::new(false));
            let worker_route = route.clone();
            let worker_path = worker_route.path.clone();
            let worker_function_name = function_name.to_owned();
            let worker_engine = runtime.metered_engine.clone();
            let worker_config = runtime.config.clone();
            let worker_telemetry = telemetry.clone();
            let worker_limits = Arc::clone(&runtime.concurrency_limits);
            let worker_host_identity = Arc::clone(&host_identity);
            let worker_storage_broker = Arc::clone(&storage_broker);
            let worker_route_overrides = Arc::clone(&route_overrides);
            let worker_peer_capabilities = Arc::clone(&peer_capabilities);
            let worker_host_capabilities = host_capabilities;
            let worker_host_load = Arc::clone(&host_load);
            let worker_stop = Arc::clone(&stop_requested);
            let join_handle = tokio::task::spawn_blocking(move || {
                run_background_tick_loop(
                    worker_engine,
                    worker_config,
                    worker_telemetry,
                    worker_limits,
                    worker_host_identity,
                    worker_storage_broker,
                    worker_route_overrides,
                    worker_peer_capabilities,
                    worker_host_capabilities,
                    worker_host_load,
                    worker_route,
                    worker_function_name,
                    worker_stop,
                )
            });

            new_workers.push(BackgroundWorkerHandle {
                route_path: worker_path,
                stop_requested,
                join_handle,
            });
            started_workers = started_workers.saturating_add(1);
        }

        if started_workers > 0 {
            tracing::info!(
                workers = started_workers,
                "started autoscaling background workers"
            );
        }

        self.workers
            .lock()
            .expect("background worker list should not be poisoned")
            .extend(new_workers);
    }

    #[cfg_attr(not(any(unix, test)), allow(dead_code))]
    #[allow(clippy::too_many_arguments)]
    async fn replace_with(
        &self,
        runtime: &RuntimeState,
        telemetry: TelemetryHandle,
        host_identity: Arc<HostIdentity>,
        storage_broker: Arc<StorageBrokerManager>,
        route_overrides: Arc<ArcSwap<HashMap<String, String>>>,
        peer_capabilities: PeerCapabilityCache,
        host_capabilities: Capabilities,
        host_load: Arc<HostLoadCounters>,
    ) {
        self.stop_all().await;
        self.start_for_runtime(
            runtime,
            telemetry,
            host_identity,
            storage_broker,
            route_overrides,
            peer_capabilities,
            host_capabilities,
            host_load,
        );
    }

    async fn stop_all(&self) {
        let workers = {
            let mut guard = self
                .workers
                .lock()
                .expect("background worker list should not be poisoned");
            std::mem::take(&mut *guard)
        };

        for worker in &workers {
            worker.stop_requested.store(true, Ordering::Release);
        }

        for worker in workers {
            if let Err(error) = worker.join_handle.await {
                tracing::warn!(
                    route = %worker.route_path,
                    "background worker task exited unexpectedly: {error}"
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_background_tick_loop(
    engine: Engine,
    config: IntegrityConfig,
    telemetry: TelemetryHandle,
    concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
    host_identity: Arc<HostIdentity>,
    storage_broker: Arc<StorageBrokerManager>,
    route_overrides: Arc<ArcSwap<HashMap<String, String>>>,
    peer_capabilities: PeerCapabilityCache,
    host_capabilities: Capabilities,
    host_load: Arc<HostLoadCounters>,
    route: IntegrityRoute,
    function_name: String,
    stop_requested: Arc<AtomicBool>,
) {
    let mut runner = match BackgroundTickRunner::new(
        &engine,
        &config,
        &route,
        &function_name,
        telemetry,
        concurrency_limits,
        host_identity,
        storage_broker,
        route_overrides,
        peer_capabilities,
        host_capabilities,
        host_load,
    ) {
        Ok(runner) => runner,
        Err(error) => {
            error.log_if_needed(&function_name);
            return;
        }
    };

    loop {
        if !wait_for_background_tick(&stop_requested) {
            break;
        }

        tracing::info!(
            route = %runner.route_path,
            function = %runner.function_name,
            "invoking autoscaling background tick"
        );
        if let Err(error) = runner.tick() {
            error.log_if_needed(&runner.function_name);
        }
    }
}

fn wait_for_background_tick(stop_requested: &AtomicBool) -> bool {
    let deadline = Instant::now() + AUTOSCALING_TICK_INTERVAL;

    loop {
        if stop_requested.load(Ordering::Acquire) {
            return false;
        }

        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return true;
        }

        std::thread::sleep(remaining.min(Duration::from_millis(100)));
    }
}
