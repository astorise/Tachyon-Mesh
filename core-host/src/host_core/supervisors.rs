#[cfg(unix)]
fn spawn_reload_watcher(state: AppState) {
    tokio::spawn(async move {
        let mut hangup = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
        {
            Ok(signal) => signal,
            Err(error) => {
                tracing::warn!("failed to install SIGHUP watcher: {error}");
                return;
            }
        };

        while hangup.recv().await.is_some() {
            if let Err(error) = reload_runtime_from_disk(&state).await {
                tracing::error!(
                    manifest = %state.manifest_path.display(),
                    "hot reload failed: {error:#}"
                );
            }
        }
    });
}

#[cfg(not(unix))]
fn spawn_reload_watcher(_state: AppState) {}

const MANIFEST_FILE_WATCHER_DEBOUNCE: Duration = Duration::from_millis(250);

/// Spawn a file watcher that triggers a hot reload whenever the integrity manifest is
/// modified or atomically replaced on disk. Many editors and CI/CD tools save the file
/// by writing a temp file and renaming it over the original, so the watcher is set up
/// against the manifest's parent directory and filters by filename rather than watching
/// the inode directly.
///
/// Triggers are coalesced over a short debounce window so that a flurry of OS events
/// (typical of atomic-rename saves) results in a single reload attempt. Validation
/// errors are absorbed by the existing `reload_runtime_from_disk` path, which logs and
/// keeps the previous runtime active.
fn spawn_manifest_file_watcher(state: AppState) {
    let manifest_path = state.manifest_path.clone();
    let Some(parent) = manifest_path.parent().map(Path::to_path_buf) else {
        tracing::warn!(
            manifest = %manifest_path.display(),
            "skipping manifest file watcher: manifest has no parent directory",
        );
        return;
    };
    let Some(target_filename) = manifest_path.file_name().map(|name| name.to_os_string()) else {
        tracing::warn!(
            manifest = %manifest_path.display(),
            "skipping manifest file watcher: manifest path lacks a final component",
        );
        return;
    };

    let (event_tx, mut event_rx) = mpsc::channel::<()>(8);

    let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        match res {
            Ok(event) => {
                let touches_manifest = event
                    .paths
                    .iter()
                    .any(|path| path.file_name() == Some(target_filename.as_os_str()));
                if !touches_manifest {
                    return;
                }
                // Use try_send so a flood of OS events cannot back-pressure the
                // notify worker thread; we only need to know "something changed".
                let _ = event_tx.try_send(());
            }
            Err(error) => {
                tracing::warn!("manifest file watcher error: {error}");
            }
        }
    });

    let mut watcher = match watcher {
        Ok(watcher) => watcher,
        Err(error) => {
            tracing::warn!(
                manifest = %manifest_path.display(),
                "failed to initialize manifest file watcher: {error}",
            );
            return;
        }
    };

    if let Err(error) =
        notify::Watcher::watch(&mut watcher, &parent, notify::RecursiveMode::NonRecursive)
    {
        tracing::warn!(
            directory = %parent.display(),
            "failed to start watching manifest directory: {error}",
        );
        return;
    }

    tokio::spawn(async move {
        // Keep the watcher alive for the lifetime of the task. Dropping it would
        // unsubscribe from the OS event source.
        let _watcher_guard = watcher;

        while event_rx.recv().await.is_some() {
            // Debounce: drain any pile-up of events that arrived during the wait.
            tokio::time::sleep(MANIFEST_FILE_WATCHER_DEBOUNCE).await;
            while event_rx.try_recv().is_ok() {}

            if let Err(error) = reload_runtime_from_disk(&state).await {
                tracing::error!(
                    manifest = %state.manifest_path.display(),
                    "manifest file watcher: hot reload failed (previous runtime preserved): {error:#}",
                );
            }
        }
    });
}

/// How often the authz-purge subscriber polls the outbox. 250 ms keeps revocation
/// latency well under one second while costing essentially nothing — the table is
/// usually empty, in which case the txn returns immediately with no rows.
const AUTHZ_PURGE_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Maximum events drained in a single poll tick. A larger batch is fine (we just
/// pop them all into the cache predicate) but bounding it keeps the txn short
/// and avoids starving other readers under a sudden burst of revocations.
const AUTHZ_PURGE_BATCH_LIMIT: usize = 64;

/// Drain the `authz_purge_outbox` table on a steady cadence, evict matching
/// entries from the in-process `AuthDecisionCache`, and delete the row only after
/// the eviction succeeds. The combined effect is at-most-five-minute (cache TTL)
/// worst-case stale access in the absence of revocations, and sub-second
/// revocation propagation in the presence of them.
fn spawn_authz_purge_subscriber(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(AUTHZ_PURGE_POLL_INTERVAL);
        loop {
            interval.tick().await;
            let core_store = Arc::clone(&state.core_store);
            let cache = state.auth_manager.decision_cache().clone();
            let drain_result = tokio::task::spawn_blocking(move || -> Result<usize> {
                let rows = core_store
                    .peek_outbox(store::CoreStoreBucket::AuthzPurgeOutbox, AUTHZ_PURGE_BATCH_LIMIT)
                    .context("failed to peek authz purge outbox")?;
                let mut applied = 0usize;
                for (key, payload) in rows {
                    match serde_json::from_slice::<auth::AuthzPurgeEvent>(&payload) {
                        Ok(event) => {
                            if let Err(error) = auth::apply_authz_purge(&cache, &event) {
                                tracing::warn!(
                                    "authz purge event `{key}` ignored due to apply failure: {error:#}"
                                );
                            } else {
                                applied += 1;
                            }
                        }
                        Err(error) => {
                            tracing::warn!(
                                "authz purge event `{key}` ignored due to parse failure: {error:#}"
                            );
                        }
                    }
                    if let Err(error) =
                        core_store.delete(store::CoreStoreBucket::AuthzPurgeOutbox, &key)
                    {
                        tracing::warn!(
                            "authz purge outbox cleanup for `{key}` failed: {error:#}"
                        );
                    }
                }
                Ok(applied)
            })
            .await;

            match drain_result {
                Ok(Ok(0)) => {} // Common case: no events to apply.
                Ok(Ok(n)) => {
                    tracing::debug!("authz purge subscriber applied {n} event(s)");
                }
                Ok(Err(error)) => {
                    tracing::warn!("authz purge subscriber drain failed: {error:#}");
                }
                Err(error) => {
                    tracing::warn!("authz purge subscriber task join failed: {error}");
                }
            }
        }
    });
}

fn spawn_volume_gc_sweeper(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(VOLUME_GC_TICK_INTERVAL);

        loop {
            interval.tick().await;
            let runtime = state.runtime.load_full();
            if let Err(error) = run_volume_gc_tick(runtime).await {
                tracing::warn!("volume GC sweep failed: {error:#}");
            }
        }
    });
}

fn spawn_buffered_request_replayer(state: AppState) {
    tokio::spawn(async move {
        loop {
            state.buffered_requests.notify.notified().await;

            loop {
                if state.buffered_requests.pending_count() == 0 {
                    break;
                }

                let runtime = state.runtime.load_full();
                if telemetry::active_requests(&state.telemetry)
                    >= PRESSURE_SATURATED_ACTIVE_REQUEST_THRESHOLD
                {
                    tokio::time::sleep(BUFFER_REPLAY_RETRY_INTERVAL).await;
                    continue;
                }

                let Some(buffered) = state.buffered_requests.pop_next().unwrap_or_else(|error| {
                    tracing::warn!("failed to load buffered request: {error}");
                    None
                }) else {
                    break;
                };

                let Some(route) = runtime
                    .config
                    .sealed_route(&buffered.request.route_path)
                    .cloned()
                else {
                    state.buffered_requests.complete(
                        buffered,
                        Err((
                            StatusCode::SERVICE_UNAVAILABLE,
                            "buffered route is no longer sealed".to_owned(),
                        )),
                    );
                    continue;
                };
                let Some(semaphore) = runtime.concurrency_limits.get(&route.path).cloned() else {
                    state.buffered_requests.complete(
                        buffered,
                        Err((
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "buffered route is missing a concurrency limiter".to_owned(),
                        )),
                    );
                    continue;
                };

                let permit = match Arc::clone(&semaphore.semaphore).try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(TryAcquireError::NoPermits) => {
                        let _ = state.buffered_requests.requeue_front(buffered);
                        tokio::time::sleep(BUFFER_REPLAY_RETRY_INTERVAL).await;
                        continue;
                    }
                    Err(TryAcquireError::Closed) => {
                        state.buffered_requests.complete(
                            buffered,
                            Err((
                                StatusCode::SERVICE_UNAVAILABLE,
                                format!("route `{}` is currently unavailable", route.path),
                            )),
                        );
                        continue;
                    }
                };

                let result = execute_buffered_route_request(
                    &state,
                    &runtime,
                    &route,
                    semaphore,
                    permit,
                    buffered.request.clone(),
                )
                .await;
                state.buffered_requests.complete(buffered, result);
            }
        }
    });
}

fn spawn_global_memory_governor(state: AppState) {
    let governor = Arc::clone(&state.memory_governor);
    let runtime = Arc::clone(&state.runtime);
    memory_governor::spawn_memory_governor(governor, move |pressure| {
        let active_runtime = runtime.load();
        active_runtime.instance_pool.invalidate_all();
        active_runtime.instance_pool.run_pending_tasks();
        tracing::warn!(
            ?pressure,
            "global memory governor evicted warm instance pool entries"
        );
    });
}

fn spawn_pressure_monitor(state: AppState) {
    tokio::spawn(async move {
        let mut previous_state = PeerPressureState::Idle;
        loop {
            let peer_count = state.uds_fast_path.active_peer_count();
            if peer_count == 0 {
                tokio::time::sleep(PRESSURE_MONITOR_IDLE_SLEEP_INTERVAL).await;
                continue;
            }

            let runtime = state.runtime.load_full();
            let active_requests = telemetry::active_requests(&state.telemetry);
            let pending_requests = runtime
                .concurrency_limits
                .values()
                .map(|control| control.pending_queue_size() as usize)
                .sum::<usize>();
            let saturated_entry = active_requests >= PRESSURE_SATURATED_ACTIVE_REQUEST_THRESHOLD
                || pending_requests >= PRESSURE_CAUTION_ACTIVE_REQUEST_THRESHOLD;
            let saturated_exit = active_requests
                < PRESSURE_SATURATED_ACTIVE_REQUEST_THRESHOLD
                    .saturating_sub(PRESSURE_CAUTION_ACTIVE_REQUEST_THRESHOLD)
                && pending_requests == 0;
            let caution_entry = active_requests >= PRESSURE_CAUTION_ACTIVE_REQUEST_THRESHOLD
                || pending_requests > 0;
            let caution_exit = active_requests
                < (PRESSURE_CAUTION_ACTIVE_REQUEST_THRESHOLD / 2).max(1)
                && pending_requests == 0;
            let pressure_state = match previous_state {
                PeerPressureState::Saturated if !saturated_exit => PeerPressureState::Saturated,
                PeerPressureState::Caution if !caution_exit && !saturated_entry => {
                    PeerPressureState::Caution
                }
                _ if saturated_entry => PeerPressureState::Saturated,
                _ if caution_entry => PeerPressureState::Caution,
                _ => PeerPressureState::Idle,
            };
            let now_unix_ms = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
                .unwrap_or_default();
            if let Err(error) = state
                .uds_fast_path
                .write_local_pressure_state(pressure_state, now_unix_ms)
            {
                tracing::debug!("failed to update local pressure metadata: {error:#}");
            }
            previous_state = pressure_state;
            tokio::time::sleep(PRESSURE_MONITOR_POLL_INTERVAL).await;
        }
    });
}

fn spawn_draining_runtime_reaper(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(DRAINING_REAPER_TICK_INTERVAL);

        loop {
            interval.tick().await;
            run_draining_runtime_reaper_tick(&state);
        }
    });
}

fn run_draining_runtime_reaper_tick(state: &AppState) {
    let now = Instant::now();
    let mut draining_runtimes = state
        .draining_runtimes
        .lock()
        .expect("draining runtime list should not be poisoned");
    let mut retained = Vec::with_capacity(draining_runtimes.len());

    for draining in draining_runtimes.drain(..) {
        let active_requests = draining.runtime.active_request_count();
        let timed_out =
            now.saturating_duration_since(draining.draining_since) >= DRAINING_ROUTE_TIMEOUT;
        if active_requests == 0 || timed_out {
            if timed_out && active_requests > 0 {
                for control in draining.runtime.concurrency_limits.values() {
                    control.force_terminate();
                }
            }

            tracing::info!(
                active_requests,
                forced = timed_out && active_requests > 0,
                drained_routes = draining.runtime.draining_route_count(),
                "graceful draining reaped an inactive runtime generation"
            );
            continue;
        }

        retained.push(draining);
    }

    *draining_runtimes = retained;
}

#[cfg_attr(not(any(unix, test)), allow(dead_code))]
async fn reload_runtime_from_disk(state: &AppState) -> Result<()> {
    let manifest_path = state.manifest_path.clone();
    let runtime = tokio::task::spawn_blocking(move || {
        let config = load_integrity_config_from_manifest_path(&manifest_path)?;
        build_runtime_state(config)
    })
    .await
    .context("hot reload task failed")??;
    prewarm_runtime_routes(
        &runtime,
        state.telemetry.clone(),
        Arc::clone(&state.host_identity),
        Arc::clone(&state.storage_broker),
    )?;
    let previous_runtime = state.runtime.load_full();
    let draining_since = Instant::now();
    previous_runtime.mark_draining(draining_since);
    state
        .draining_runtimes
        .lock()
        .expect("draining runtime list should not be poisoned")
        .push(DrainingRuntime {
            runtime: previous_runtime,
            draining_since,
        });

    state
        .background_workers
        .replace_with(
            &runtime,
            state.telemetry.clone(),
            Arc::clone(&state.host_identity),
            Arc::clone(&state.storage_broker),
            Arc::clone(&state.route_overrides),
            Arc::clone(&state.peer_capabilities),
            state.host_capabilities,
            Arc::clone(&state.host_load),
        )
        .await;
    let runtime = Arc::new(runtime);
    state.runtime.store(Arc::clone(&runtime));
    run_draining_runtime_reaper_tick(state);
    tracing::info!(
        manifest = %state.manifest_path.display(),
        draining_generations = state
            .draining_runtimes
            .lock()
            .expect("draining runtime list should not be poisoned")
            .len(),
        "Hot reload successful"
    );
    Ok(())
}

fn secure_cache_bootstrap(core_store: &store::CoreStore, runtime: &RuntimeState) -> Result<()> {
    let engine_hash = runtime_engine_cache_hash(runtime);
    core_store.secure_cwasm_cache_bootstrap(&engine_hash)?;
    Ok(())
}

fn runtime_engine_cache_hash(runtime: &RuntimeState) -> String {
    format!(
        "{}:{}",
        engine_precompile_hash_string(&runtime.engine),
        engine_precompile_hash_string(&runtime.metered_engine)
    )
}

fn engine_precompile_hash_string(engine: &Engine) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    engine.precompile_compatibility_hash().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

async fn maybe_run_bootstrap_mode(config: &IntegrityConfig) -> Result<bool> {
    if !env_flag(BOOTSTRAP_IF_UNENROLLED_ENV) || has_enrollment_credentials() {
        return Ok(false);
    }

    let endpoint = config.enrollment_endpoint.as_deref().ok_or_else(|| {
        anyhow!("bootstrap mode requested but sealed config does not define `enrollment_endpoint`")
    })?;
    let cert_output_path = std::env::var(ENROLLMENT_CERT_PATH_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("auth-state/enrolled-node.cert"));
    tracing::warn!("Entering Bootstrap Mode: isolating host startup to system-faas-enrollment");
    system_faas_enrollment::run_enrollment(system_faas_enrollment::EnrollmentConfig {
        bootstrap_url: endpoint.to_owned(),
        cert_output_path,
        poll_interval: Duration::from_secs(30),
        max_polls: 120,
    })
    .await?;
    Ok(true)
}

fn has_enrollment_credentials() -> bool {
    if std::env::var_os(NODE_CERT_PEM_ENV).is_some() && std::env::var_os(NODE_KEY_PEM_ENV).is_some()
    {
        return true;
    }
    std::env::var(ENROLLMENT_CERT_PATH_ENV)
        .ok()
        .map(|path| Path::new(&path).is_file())
        .unwrap_or(false)
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let ctrl_c = tokio::signal::ctrl_c();
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = ctrl_c => {}
                    _ = terminate.recv() => {}
                }
            }
            Err(error) => {
                tracing::warn!("failed to install SIGTERM watcher: {error}");
                let _ = ctrl_c.await;
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }

    tracing::info!("shutdown signal received");
}
