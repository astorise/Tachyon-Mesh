use super::*;

#[cfg(unix)]
pub(crate) fn spawn_reload_watcher(state: AppState) {
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
pub(crate) fn spawn_reload_watcher(_state: AppState) {}

pub(crate) const MANIFEST_FILE_WATCHER_DEBOUNCE: Duration = Duration::from_millis(250);

/// Whether a filesystem event on the manifest directory should be ignored by the
/// hot-reload watcher. Pure access/open events do not change the file's contents
/// (a `/admin/manifest` update *writes* it), and on some filesystems every read —
/// including the periodic S3 backup flush reading `integrity.lock` — emits one;
/// reacting to those re-armed the watcher into an infinite reload loop.
fn watcher_event_is_ignorable(kind: &notify::EventKind) -> bool {
    matches!(kind, notify::EventKind::Access(_))
}

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
pub(crate) fn spawn_manifest_file_watcher(state: AppState) {
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
                // Ignore pure access/open events. On some filesystems (notably the
                // WSL2/k3s `local-path` PVC backing `/data`) every *read* of the
                // manifest emits an `Access(Open)` event — including the periodic
                // S3 backup flush reading `integrity.lock` to upload it. Treating
                // those as changes triggered a hot reload that re-read the file,
                // which emitted another access event, sustaining an infinite
                // reload loop. Only react to events that can alter the contents
                // (Create/Modify/Remove/rename); a real `/admin/manifest` update
                // writes the file, so legitimate hot reloads are unaffected.
                if watcher_event_is_ignorable(&event.kind) {
                    return;
                }
                let touches_manifest = event
                    .paths
                    .iter()
                    .any(|path| path.file_name() == Some(target_filename.as_os_str()));
                if !touches_manifest {
                    return;
                }
                tracing::info!(
                    event_kind = ?event.kind,
                    paths = ?event.paths,
                    "manifest file watcher: change detected, scheduling hot reload"
                );
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
pub(crate) const AUTHZ_PURGE_POLL_INTERVAL: Duration = Duration::from_millis(250);

/// Maximum events drained in a single poll tick. A larger batch is fine (we just
/// pop them all into the cache predicate) but bounding it keeps the txn short
/// and avoids starving other readers under a sudden burst of revocations.
pub(crate) const AUTHZ_PURGE_BATCH_LIMIT: usize = 64;

/// Drain the `authz_purge_outbox` table on a steady cadence, evict matching
/// entries from the in-process `AuthDecisionCache`, and delete the row only after
/// the eviction succeeds. The combined effect is at-most-five-minute (cache TTL)
/// worst-case stale access in the absence of revocations, and sub-second
/// revocation propagation in the presence of them.
pub(crate) fn spawn_authz_purge_subscriber(state: AppState) {
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

/// How often the config-gossip bridge drains the `config_update_outbox` and
/// announces accepted manifests to peers. Config changes are rare, so a relaxed
/// cadence keeps the (normally empty) read essentially free.
pub(crate) const CONFIG_GOSSIP_POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Maximum config-update events broadcast in a single poll tick.
pub(crate) const CONFIG_GOSSIP_BATCH_LIMIT: usize = 32;

const MESH_OVERLAY_ROUTE_NAME: &str = "system-faas-mesh-overlay";
const MESH_OVERLAY_DEFAULT_MOUNT: &str = "/system/mesh-overlay";
const MESH_OVERLAY_CONFIG_UPDATE_SUBPATH: &str = "/config/update";
const OVERLAY_PEER_URLS_ENV: &str = "PEER_URLS";
const OVERLAY_SHARED_SECRET_ENV: &str = "OVERLAY_SHARED_SECRET";
const OVERLAY_NODE_ID_ENV: &str = "NODE_ID";
const DEFAULT_OVERLAY_NODE_ID: &str = "local-node";
const OVERLAY_AUTH_HEADER: &str = "x-tachyon-overlay-auth";

/// Per-peer announcement timeout. The gossip subscriber is single-threaded, so a
/// peer that accepts the connection but never sends response headers would
/// otherwise block every later peer and every later outbox row indefinitely.
const CONFIG_GOSSIP_PEER_TIMEOUT: Duration = Duration::from_secs(5);

/// Where to fan out `ConfigUpdateEvent`s. The host keeps no standalone peer
/// registry — peer URLs and the overlay shared secret live in the `mesh-overlay`
/// route's `env`, the very same values the `system-faas-mesh-overlay` guest
/// consumes for discovery — so we read them straight from the live config.
struct ConfigGossipTargets {
    /// Peer base URLs (scheme + authority), e.g. `https://node-b:8443`.
    peers: Vec<String>,
    /// Mount path of the mesh-overlay route on every peer host.
    mount_path: String,
    /// Shared secret the peer's `authorize_peer` expects, when configured.
    auth_secret: Option<String>,
    /// This node's overlay `NODE_ID` — the key peers index it under in their
    /// routing table (from its heartbeats), and therefore the value
    /// `origin_node_id` must carry so a peer can resolve the origin and pull.
    node_id: String,
}

/// Drain the `config_update_outbox` on a steady cadence and announce each
/// accepted manifest to peers by POSTing the `ConfigUpdateEvent` to their
/// `system-faas-mesh-overlay` `/config/update` endpoint. Peers compare the
/// advertised version with their own and pull the full manifest from the origin
/// node over the secure overlay only when they are behind.
///
/// This is the host-side half of multi-master config sync: `core-host` already
/// writes the durable outbox row and fires the in-process `config_updates`
/// broadcast when it accepts a manifest (see `integrity_config.rs`); this
/// subscriber turns those rows into the cross-node gossip the
/// distributed-control-plane spec requires. Delivery is best-effort — the
/// receiver is version-guarded and idempotent, and peers also self-heal through
/// their own overlay discovery — so a row is removed after its broadcast round to
/// keep the outbox bounded.
pub(crate) fn spawn_config_gossip_subscriber(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CONFIG_GOSSIP_POLL_INTERVAL);
        loop {
            interval.tick().await;
            if let Err(error) = drain_config_update_outbox(&state).await {
                tracing::warn!("config gossip subscriber drain failed: {error:#}");
            }
        }
    });
}

async fn drain_config_update_outbox(state: &AppState) -> Result<()> {
    let core_store = Arc::clone(&state.core_store);
    let rows = tokio::task::spawn_blocking(move || {
        core_store
            .peek_outbox(
                store::CoreStoreBucket::ConfigUpdateOutbox,
                CONFIG_GOSSIP_BATCH_LIMIT,
            )
            .context("failed to peek config update outbox")
    })
    .await
    .context("config gossip peek task join failed")??;

    if rows.is_empty() {
        return Ok(());
    }

    let targets = config_gossip_targets(&state.runtime.load().config.routes);
    let mut acked = 0usize;
    let mut keys = Vec::with_capacity(rows.len());
    for (key, payload) in rows {
        if let Some(targets) = &targets {
            let outgoing = stamp_origin_node_id(&payload, &targets.node_id);
            acked += broadcast_config_event(state, targets, &outgoing).await;
        }
        keys.push(key);
    }

    // Remove the drained rows regardless of per-peer delivery outcome: the
    // broadcast is best-effort gossip, and retaining rows because of a single
    // unreachable peer would grow the outbox without bound.
    let drained = keys.len();
    let core_store = Arc::clone(&state.core_store);
    tokio::task::spawn_blocking(move || {
        for key in keys {
            if let Err(error) = core_store.delete(store::CoreStoreBucket::ConfigUpdateOutbox, &key)
            {
                tracing::warn!("config_update_outbox cleanup for `{key}` failed: {error:#}");
            }
        }
    })
    .await
    .context("config gossip cleanup task join failed")?;

    if targets.is_none() {
        tracing::debug!(
            "config gossip: drained {drained} event(s) with no mesh-overlay peers configured"
        );
    } else {
        tracing::debug!("config gossip: announced {drained} event(s), {acked} peer ack(s)");
    }
    Ok(())
}

/// POST one serialized `ConfigUpdateEvent` to every peer's mesh-overlay
/// `/config/update` endpoint. Returns the number of peers that accepted it.
async fn broadcast_config_event(
    state: &AppState,
    targets: &ConfigGossipTargets,
    payload: &[u8],
) -> usize {
    let mut acked = 0usize;
    for peer in &targets.peers {
        let url = format!(
            "{}{}{}",
            peer.trim_end_matches('/'),
            targets.mount_path,
            MESH_OVERLAY_CONFIG_UPDATE_SUBPATH
        );
        let mut request = state
            .http_client
            .post(&url)
            .timeout(CONFIG_GOSSIP_PEER_TIMEOUT)
            .header("content-type", "application/json")
            .body(payload.to_vec());
        if let Some(secret) = &targets.auth_secret {
            request = request.header(OVERLAY_AUTH_HEADER, secret);
        }
        match request.send().await {
            Ok(response) if response.status().is_success() => {
                acked += 1;
                tracing::debug!(peer = %peer, status = %response.status(), "config update announced");
            }
            Ok(response) => {
                tracing::warn!(
                    peer = %peer,
                    status = %response.status(),
                    "peer rejected config update announcement"
                );
            }
            Err(error) => {
                tracing::warn!(peer = %peer, "failed to announce config update: {error}");
            }
        }
    }
    acked
}

/// Resolve peer fan-out targets from the live `mesh-overlay` route. Returns
/// `None` when the route is absent or declares no peers (e.g. a single-node
/// deployment), in which case there is nothing to propagate.
fn config_gossip_targets(routes: &[IntegrityRoute]) -> Option<ConfigGossipTargets> {
    let route = routes.iter().find(|route| {
        route.name == MESH_OVERLAY_ROUTE_NAME || route.path == MESH_OVERLAY_DEFAULT_MOUNT
    })?;
    let peers = route
        .env
        .get(OVERLAY_PEER_URLS_ENV)
        .map(|raw| {
            raw.split(',')
                .map(|entry| entry.trim().trim_end_matches('/').to_owned())
                .filter(|entry| !entry.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if peers.is_empty() {
        return None;
    }
    let mount_path = if route.path.trim().is_empty() {
        MESH_OVERLAY_DEFAULT_MOUNT.to_owned()
    } else {
        route.path.trim_end_matches('/').to_owned()
    };
    let auth_secret = route
        .env
        .get(OVERLAY_SHARED_SECRET_ENV)
        .map(|secret| secret.trim().to_owned())
        .filter(|secret| !secret.is_empty());
    let node_id = route
        .env
        .get(OVERLAY_NODE_ID_ENV)
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| DEFAULT_OVERLAY_NODE_ID.to_owned());
    Some(ConfigGossipTargets {
        peers,
        mount_path,
        auth_secret,
        node_id,
    })
}

/// Rewrite a stored event's `origin_node_id` to the value peers index this node
/// under (its overlay heartbeat `NODE_ID`). The admin handler stamps the event
/// with the host public key, but `system-faas-mesh-overlay` resolves the origin
/// peer by heartbeat `NODE_ID`; without this rewrite `pull_config_update` returns
/// `404 origin peer unknown` and the manifest never propagates. Non-event rows
/// are forwarded unchanged.
fn stamp_origin_node_id(payload: &[u8], node_id: &str) -> Vec<u8> {
    match serde_json::from_slice::<ConfigUpdateEvent>(payload) {
        Ok(mut event) => {
            event.origin_node_id = node_id.to_owned();
            serde_json::to_vec(&event).unwrap_or_else(|_| payload.to_vec())
        }
        Err(_) => payload.to_vec(),
    }
}

pub(crate) fn spawn_volume_gc_sweeper(state: AppState) {
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

pub(crate) fn spawn_buffered_request_replayer(state: AppState) {
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

pub(crate) fn spawn_global_memory_governor(state: AppState) {
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

pub(crate) fn spawn_pressure_monitor(state: AppState) {
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

pub(crate) fn spawn_draining_runtime_reaper(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(DRAINING_REAPER_TICK_INTERVAL);

        loop {
            interval.tick().await;
            run_draining_runtime_reaper_tick(&state);
        }
    });
}

pub(crate) fn run_draining_runtime_reaper_tick(state: &AppState) {
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
pub(crate) async fn reload_runtime_from_disk(state: &AppState) -> Result<()> {
    let manifest_path = state.manifest_path.clone();
    let current_trusted = state.runtime.load().config.trusted_signers.clone();
    let runtime = tokio::task::spawn_blocking(move || {
        let config = load_integrity_config_from_manifest_path_with_trusted(
            &manifest_path,
            &current_trusted,
        )?;
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
    spawn_canary_evaluators(&runtime.config);
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

pub(crate) fn secure_cache_bootstrap(
    core_store: &store::CoreStore,
    runtime: &RuntimeState,
) -> Result<()> {
    let engine_hash = runtime_engine_cache_hash(runtime);
    core_store.secure_cwasm_cache_bootstrap(&engine_hash)?;
    Ok(())
}

pub(crate) fn runtime_engine_cache_hash(runtime: &RuntimeState) -> String {
    format!(
        "{}:{}",
        engine_precompile_hash_string(&runtime.engine),
        engine_precompile_hash_string(&runtime.metered_engine)
    )
}

pub(crate) fn engine_precompile_hash_string(engine: &Engine) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    engine.precompile_compatibility_hash().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub(crate) async fn maybe_run_bootstrap_mode(config: &IntegrityConfig) -> Result<bool> {
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
    let identity_token_path = std::env::var("TACHYON_ENROLLMENT_IDENTITY_TOKEN_PATH")
        .ok()
        .map(PathBuf::from);
    system_faas_enrollment::run_enrollment(system_faas_enrollment::EnrollmentConfig {
        bootstrap_url: endpoint.to_owned(),
        cert_output_path,
        poll_interval: Duration::from_secs(30),
        max_polls: 120,
        identity_token_path,
    })
    .await?;
    Ok(true)
}

pub(crate) fn has_enrollment_credentials() -> bool {
    if std::env::var_os(NODE_CERT_PEM_ENV).is_some() && std::env::var_os(NODE_KEY_PEM_ENV).is_some()
    {
        return true;
    }
    std::env::var(ENROLLMENT_CERT_PATH_ENV)
        .ok()
        .map(|path| Path::new(&path).is_file())
        .unwrap_or(false)
}

pub(crate) fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

pub(crate) async fn shutdown_signal() {
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

// ── Volume backup scheduler ───────────────────────────────────────────────────

/// Spawn a background task that checks every 60 seconds whether any volume has
/// a `backup_schedule` cron that is due and triggers a backup if so.
pub(crate) fn spawn_volume_backup_scheduler(state: AppState) {
    tokio::spawn(async move {
        // Track last backup time per (route_path, guest_path) to avoid
        // triggering more than once per cron window.
        let mut last_backup: std::collections::HashMap<(String, String), u64> =
            std::collections::HashMap::new();

        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            let config = state.runtime.load().config.clone();
            let now_mins = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
                / 60;

            for route in &config.routes {
                for volume in &route.volumes {
                    let Some(ref schedule) = volume.backup_schedule else {
                        continue;
                    };
                    let coordination = schedule.coordination();
                    // ManualOnly disables scheduled triggers entirely.
                    if coordination == BackupCoordination::ManualOnly {
                        continue;
                    }
                    let key = (route.path.clone(), volume.guest_path.clone());
                    let last = *last_backup.get(&key).unwrap_or(&0);
                    if !cron_is_due(schedule.cron(), now_mins, last) {
                        continue;
                    }
                    // MeshLeader: only the deterministically elected leader runs the backup.
                    if coordination == BackupCoordination::MeshLeader {
                        let leader_key = format!("{}:{}", route.path, volume.guest_path);
                        if !leader_election::am_i_leader(&state, &leader_key) {
                            continue;
                        }
                    }
                    last_backup.insert(key, now_mins);
                    let config2 = config.clone();
                    let route_path = route.path.clone();
                    let guest_path = volume.guest_path.clone();
                    let write_isolation = schedule.write_isolation();
                    let state2 = state.clone();
                    tokio::spawn(async move {
                        // Drain mode pauses admission and waits for in-flight invocations.
                        if write_isolation == WriteIsolation::Drain {
                            crate::host_core::concurrency_admission::backup_drain::pause_admission(
                                &state2,
                                &route_path,
                            )
                            .await;
                            crate::host_core::concurrency_admission::backup_drain::wait_for_drain(
                                &state2,
                                &route_path,
                            )
                            .await;
                        }
                        let result = volume_backup::backup_volume(
                            &config2,
                            &route_path,
                            &guest_path,
                            write_isolation,
                        )
                        .await;
                        if write_isolation == WriteIsolation::Drain {
                            crate::host_core::concurrency_admission::backup_drain::resume_admission(
                                &state2,
                                &route_path,
                            );
                        }
                        match result {
                            Ok(snap) => tracing::info!(
                                route = %route_path,
                                guest_path = %guest_path,
                                snapshot_id = %snap.snapshot_id,
                                "scheduled volume backup completed"
                            ),
                            Err(error) => tracing::warn!(
                                route = %route_path,
                                guest_path = %guest_path,
                                "scheduled volume backup failed: {error:#}"
                            ),
                        }
                    });
                }
            }
        }
    });
}

/// Returns true if `schedule` (5-field cron) is due at `now_mins` (minutes since Unix epoch)
/// and has not already run this minute (`last_run_mins < now_mins`).
fn cron_is_due(schedule: &str, now_mins: u64, last_run_mins: u64) -> bool {
    if last_run_mins >= now_mins {
        return false;
    }
    let fields: Vec<&str> = schedule.split_whitespace().collect();
    if fields.len() != 5 {
        return false;
    }
    // Derive current time components from now_mins.
    let total_secs = now_mins * 60;
    let minute = (total_secs / 60) % 60;
    let hour = (total_secs / 3600) % 24;
    // Use a simple check: does the minute/hour field match?
    // Full calendar matching (dom/month/dow) is deferred to phase 2.
    cron_field_matches(fields[0], minute as u32) && cron_field_matches(fields[1], hour as u32)
}

fn cron_field_matches(field: &str, value: u32) -> bool {
    if field == "*" {
        return true;
    }
    if let Some((_, step)) = field.split_once('/') {
        return step
            .parse::<u32>()
            .is_ok_and(|step| step > 0 && value.is_multiple_of(step));
    }
    if let Ok(n) = field.parse::<u32>() {
        return n == value;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn watcher_ignores_access_events_but_not_writes() {
        use notify::event::{AccessKind, AccessMode, CreateKind, ModifyKind, RemoveKind};
        use notify::EventKind;
        // The exact event the WSL2/k3s `local-path` PVC emits on every read of the
        // manifest (e.g. the S3 backup flush) — must NOT trigger a reload.
        assert!(watcher_event_is_ignorable(&EventKind::Access(AccessKind::Open(
            AccessMode::Any
        ))));
        assert!(watcher_event_is_ignorable(&EventKind::Access(AccessKind::Read)));
        assert!(watcher_event_is_ignorable(&EventKind::Access(AccessKind::Any)));
        // Content-changing events (a real `/admin/manifest` write/rename) still do.
        assert!(!watcher_event_is_ignorable(&EventKind::Modify(ModifyKind::Any)));
        assert!(!watcher_event_is_ignorable(&EventKind::Create(CreateKind::Any)));
        assert!(!watcher_event_is_ignorable(&EventKind::Remove(RemoveKind::Any)));
    }

    fn overlay_route(env: &[(&str, &str)]) -> IntegrityRoute {
        IntegrityRoute {
            name: MESH_OVERLAY_ROUTE_NAME.to_owned(),
            path: MESH_OVERLAY_DEFAULT_MOUNT.to_owned(),
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect::<BTreeMap<_, _>>(),
            ..Default::default()
        }
    }

    #[test]
    fn config_gossip_targets_is_none_without_overlay_route() {
        let routes = vec![IntegrityRoute {
            name: "api".to_owned(),
            path: "/api/generate".to_owned(),
            ..Default::default()
        }];
        assert!(config_gossip_targets(&routes).is_none());
    }

    #[test]
    fn config_gossip_targets_is_none_when_no_peers_declared() {
        let routes = vec![overlay_route(&[("OVERLAY_SHARED_SECRET", "s3cr3t")])];
        assert!(config_gossip_targets(&routes).is_none());
    }

    #[test]
    fn config_gossip_targets_parses_peers_secret_and_node_id() {
        let routes = vec![overlay_route(&[
            ("PEER_URLS", "https://node-b:8443/ , https://node-c:8443"),
            ("OVERLAY_SHARED_SECRET", "s3cr3t"),
            ("NODE_ID", "node-a-pub"),
        ])];
        let targets = config_gossip_targets(&routes).expect("targets should resolve");
        assert_eq!(
            targets.peers,
            vec![
                "https://node-b:8443".to_owned(),
                "https://node-c:8443".to_owned(),
            ]
        );
        assert_eq!(targets.mount_path, MESH_OVERLAY_DEFAULT_MOUNT);
        assert_eq!(targets.auth_secret.as_deref(), Some("s3cr3t"));
        assert_eq!(targets.node_id, "node-a-pub");
    }

    #[test]
    fn config_gossip_targets_matches_by_path_and_defaults_node_id() {
        let mut route = overlay_route(&[("PEER_URLS", "https://node-b:8443")]);
        route.name = String::new(); // force the match to rely on the path alone
        let targets = config_gossip_targets(&[route]).expect("targets should resolve");
        assert_eq!(targets.peers, vec!["https://node-b:8443".to_owned()]);
        assert!(
            targets.auth_secret.is_none(),
            "absent secret must stay None so unauthenticated overlays still work"
        );
        assert_eq!(
            targets.node_id, DEFAULT_OVERLAY_NODE_ID,
            "node_id must mirror the overlay heartbeat default so peers can resolve the origin"
        );
    }

    #[test]
    fn stamp_origin_node_id_overrides_event_origin_and_passes_through_garbage() {
        let original =
            br#"{"version":7,"checksum":"sha256:abc","origin_node_id":"hostpubkey","ts_ms":123}"#;
        let stamped = stamp_origin_node_id(original, "node-a");
        let event: ConfigUpdateEvent = serde_json::from_slice(&stamped)
            .expect("stamped payload should still be a valid event");
        assert_eq!(event.origin_node_id, "node-a");
        assert_eq!(event.version, 7);
        assert_eq!(event.checksum, "sha256:abc");

        // Non-event rows are forwarded unchanged rather than dropped.
        assert_eq!(
            stamp_origin_node_id(b"not an event", "node-a"),
            b"not an event"
        );
    }
}
