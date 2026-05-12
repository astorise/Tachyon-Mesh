use super::*;

impl BackgroundWorkerManager {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_for_runtime(
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
    pub(crate) async fn replace_with(
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

    pub(crate) async fn stop_all(&self) {
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
pub(crate) fn run_background_tick_loop(
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

/// Stops any running canary evaluators, then spawns a new evaluator tokio task
/// for every route in `config` that carries a `canary` deployment config.
/// Must be called from an async context (i.e., after `reload_runtime_from_disk`).
pub(crate) fn spawn_canary_evaluators(config: &IntegrityConfig) {
    let registry = canary_rollouts();

    // Signal all existing evaluators to stop.
    {
        let guard = registry.lock().unwrap_or_else(|p| p.into_inner());
        for state in guard.values() {
            let _ = state.stop_tx.send(true);
        }
    }

    // Rebuild the registry from the new config.
    let mut guard = registry.lock().unwrap_or_else(|p| p.into_inner());
    guard.clear();

    for route in &config.routes {
        let Some(canary_cfg) = &route.canary else {
            continue;
        };

        let initial_weight = canary_cfg.step_weight.min(100);
        let (stop_tx, stop_rx) = watch::channel(false);
        let rollout = Arc::new(CanaryRolloutState {
            route_path: route.path.clone(),
            current_version: route.name.clone(),
            next_version: canary_cfg.next_version.clone(),
            weight_pct: AtomicU32::new(initial_weight),
            step_weight: canary_cfg.step_weight,
            interval_secs: canary_cfg.interval_secs,
            max_error_rate: canary_cfg.max_error_rate,
            phase: Mutex::new(CanaryPhase::Stepping),
            next_req_count: AtomicU64::new(0),
            next_err_count: AtomicU64::new(0),
            stop_tx,
        });

        guard.insert(route.path.clone(), Arc::clone(&rollout));
        tokio::spawn(run_canary_evaluator(rollout, stop_rx));

        tracing::info!(
            route = %route.path,
            next_version = %canary_cfg.next_version,
            initial_weight = initial_weight,
            "canary rollout started"
        );
    }
}

async fn run_canary_evaluator(
    state: Arc<CanaryRolloutState>,
    mut stop_rx: watch::Receiver<bool>,
) {
    let interval = Duration::from_secs(state.interval_secs);

    loop {
        // Wait for the configured interval or an explicit stop signal.
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            result = stop_rx.changed() => {
                if result.is_err() || *stop_rx.borrow() {
                    break;
                }
            }
        }

        // Exit if already in a terminal phase.
        {
            let phase = state.phase.lock().unwrap_or_else(|p| p.into_inner());
            if !matches!(*phase, CanaryPhase::Stepping) {
                break;
            }
        }

        // Compute cumulative error rate for next_version traffic.
        let total_req = state.next_req_count.load(Ordering::Relaxed);
        let total_err = state.next_err_count.load(Ordering::Relaxed);
        let error_rate = if total_req > 0 {
            total_err as f32 / total_req as f32
        } else {
            0.0
        };

        if error_rate > state.max_error_rate {
            // Atomic rollback: reset traffic weight and record phase.
            state.weight_pct.store(0, Ordering::SeqCst);
            let reason = format!(
                "error rate {:.1}% exceeded threshold {:.1}%",
                error_rate * 100.0,
                state.max_error_rate * 100.0,
            );
            {
                let mut phase = state.phase.lock().unwrap_or_else(|p| p.into_inner());
                *phase = CanaryPhase::RolledBack {
                    reason: reason.clone(),
                };
            }
            tracing::error!(
                route = %state.route_path,
                version = %state.next_version,
                error_rate,
                threshold = state.max_error_rate,
                "CANARY ROLLBACK: {}",
                reason,
            );
            break;
        }

        // Step up traffic weight toward 100%.
        let current = state.weight_pct.load(Ordering::Relaxed);
        let new_weight = current.saturating_add(state.step_weight).min(100);
        state.weight_pct.store(new_weight, Ordering::SeqCst);

        if new_weight >= 100 {
            let mut phase = state.phase.lock().unwrap_or_else(|p| p.into_inner());
            *phase = CanaryPhase::Promoted;
            tracing::info!(
                route = %state.route_path,
                version = %state.next_version,
                "CANARY PROMOTED: 100% traffic shifted to next version"
            );
            break;
        }

        tracing::info!(
            route = %state.route_path,
            version = %state.next_version,
            weight_pct = new_weight,
            "canary rollout step complete"
        );
    }
}

pub(crate) fn wait_for_background_tick(stop_requested: &AtomicBool) -> bool {
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
