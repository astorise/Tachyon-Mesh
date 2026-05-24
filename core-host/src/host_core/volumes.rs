use super::*;

impl VolumeManager {
    pub(crate) async fn acquire_route_volumes(
        &self,
        route: &IntegrityRoute,
        storage_broker: Arc<StorageBrokerManager>,
    ) -> std::result::Result<RouteVolumeLeaseGuard, String> {
        let mut leases = Vec::new();
        for volume in route
            .volumes
            .iter()
            .filter(|volume| volume.is_hibernation_capable())
        {
            let managed = self.managed_volume(route, volume, Arc::clone(&storage_broker))?;
            leases.push(managed.acquire().await?);
        }

        Ok(RouteVolumeLeaseGuard { leases })
    }

    pub(crate) fn managed_volume(
        &self,
        route: &IntegrityRoute,
        volume: &IntegrityVolume,
        storage_broker: Arc<StorageBrokerManager>,
    ) -> std::result::Result<Arc<ManagedVolume>, String> {
        let key = managed_volume_key(&route.path, &volume.guest_path);
        let mut volumes = self
            .volumes
            .lock()
            .expect("managed volume registry should not be poisoned");
        if let Some(volume) = volumes.get(&key) {
            return Ok(Arc::clone(volume));
        }

        let managed = Arc::new(ManagedVolume::new(&route.path, volume, storage_broker)?);
        volumes.insert(key, Arc::clone(&managed));
        Ok(managed)
    }

    #[cfg(test)]
    pub(crate) fn managed_volume_for_route(
        &self,
        route_path: &str,
        guest_path: &str,
    ) -> Option<Arc<ManagedVolume>> {
        self.volumes
            .lock()
            .expect("managed volume registry should not be poisoned")
            .get(&managed_volume_key(route_path, guest_path))
            .cloned()
    }
}

impl ManagedVolume {
    pub(crate) fn new(
        route_path: &str,
        volume: &IntegrityVolume,
        storage_broker: Arc<StorageBrokerManager>,
    ) -> std::result::Result<Self, String> {
        let active_path = normalize_path(PathBuf::from(&volume.host_path));
        fs::create_dir_all(&active_path).map_err(|error| {
            format!(
                "failed to initialize RAM volume directory `{}` for route `{route_path}`: {error}",
                active_path.display()
            )
        })?;

        Ok(Self {
            id: managed_volume_id(route_path, &volume.guest_path),
            route_path: route_path.to_owned(),
            guest_path: volume.guest_path.clone(),
            snapshot_path: snapshot_path_for_volume(&active_path),
            active_path,
            idle_timeout: volume
                .parsed_idle_timeout()
                .map_err(|error| format!("{error:#}"))?
                .ok_or_else(|| {
                    format!(
                        "route `{route_path}` volume `{}` is missing an `idle_timeout` for hibernation",
                        volume.guest_path
                    )
                })?,
            state: Mutex::new(ManagedVolumeState {
                lifecycle: ManagedVolumeLifecycle::Active,
                active_leases: 0,
                generation: 0,
            }),
            notify: Notify::new(),
            storage_broker,
        })
    }

    pub(crate) async fn acquire(
        self: &Arc<Self>,
    ) -> std::result::Result<ManagedVolumeLease, String> {
        loop {
            let should_restore = {
                let mut state = self
                    .state
                    .lock()
                    .expect("managed volume state should not be poisoned");
                match state.lifecycle {
                    ManagedVolumeLifecycle::Active => {
                        state.active_leases = state.active_leases.saturating_add(1);
                        state.generation = state.generation.saturating_add(1);
                        return Ok(ManagedVolumeLease {
                            volume: Arc::clone(self),
                        });
                    }
                    ManagedVolumeLifecycle::OnDisk => {
                        state.lifecycle = ManagedVolumeLifecycle::Hibernating;
                        state.generation = state.generation.saturating_add(1);
                        true
                    }
                    ManagedVolumeLifecycle::Hibernating => false,
                }
            };

            if should_restore {
                let completion = self.storage_broker.enqueue_restore(
                    self.id.clone(),
                    &self.active_path,
                    &self.snapshot_path,
                    &self.active_path,
                )?;
                match completion.await {
                    Ok(Ok(())) => self.finish_restore(ManagedVolumeLifecycle::Active),
                    Ok(Err(error)) => {
                        self.finish_restore(ManagedVolumeLifecycle::OnDisk);
                        return Err(format!(
                            "failed to restore hibernated volume `{}`: {error}",
                            self.id
                        ));
                    }
                    Err(_) => {
                        self.finish_restore(ManagedVolumeLifecycle::OnDisk);
                        return Err(format!(
                            "storage broker restore completion channel closed for volume `{}`",
                            self.id
                        ));
                    }
                }
                continue;
            }

            self.notify.notified().await;
        }
    }

    pub(crate) fn release(self: &Arc<Self>) {
        let generation = {
            let mut state = self
                .state
                .lock()
                .expect("managed volume state should not be poisoned");
            state.active_leases = state.active_leases.saturating_sub(1);
            state.generation = state.generation.saturating_add(1);
            if state.lifecycle == ManagedVolumeLifecycle::Active && state.active_leases == 0 {
                Some(state.generation)
            } else {
                None
            }
        };

        if let Some(generation) = generation {
            self.schedule_hibernation(generation);
        }
        self.notify.notify_waiters();
    }

    pub(crate) fn schedule_hibernation(self: &Arc<Self>, generation: u64) {
        let volume = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(volume.idle_timeout).await;

            let should_snapshot = {
                let mut state = volume
                    .state
                    .lock()
                    .expect("managed volume state should not be poisoned");
                if state.lifecycle != ManagedVolumeLifecycle::Active
                    || state.active_leases != 0
                    || state.generation != generation
                {
                    return;
                }

                state.lifecycle = ManagedVolumeLifecycle::Hibernating;
                state.generation = state.generation.saturating_add(1);
                true
            };

            if !should_snapshot {
                return;
            }

            let completion = match volume.storage_broker.enqueue_snapshot(
                volume.id.clone(),
                &volume.active_path,
                &volume.active_path,
                &volume.snapshot_path,
            ) {
                Ok(completion) => completion,
                Err(error) => {
                    tracing::warn!(
                        volume_id = %volume.id,
                        route = %volume.route_path,
                        guest_path = %volume.guest_path,
                        "failed to schedule hibernation snapshot: {error}"
                    );
                    volume.finish_restore(ManagedVolumeLifecycle::Active);
                    return;
                }
            };

            match completion.await {
                Ok(Ok(())) => volume.finish_restore(ManagedVolumeLifecycle::OnDisk),
                Ok(Err(error)) => {
                    tracing::warn!(
                        volume_id = %volume.id,
                        route = %volume.route_path,
                        guest_path = %volume.guest_path,
                        "hibernation snapshot failed: {error}"
                    );
                    volume.finish_restore(ManagedVolumeLifecycle::Active);
                }
                Err(_) => {
                    tracing::warn!(
                        volume_id = %volume.id,
                        route = %volume.route_path,
                        guest_path = %volume.guest_path,
                        "hibernation snapshot completion channel closed unexpectedly"
                    );
                    volume.finish_restore(ManagedVolumeLifecycle::Active);
                }
            }
        });
    }

    pub(crate) fn finish_restore(&self, lifecycle: ManagedVolumeLifecycle) {
        let mut state = self
            .state
            .lock()
            .expect("managed volume state should not be poisoned");
        state.lifecycle = lifecycle;
        state.generation = state.generation.saturating_add(1);
        self.notify.notify_waiters();
    }

    #[cfg(test)]
    pub(crate) fn lifecycle(&self) -> ManagedVolumeLifecycle {
        self.state
            .lock()
            .expect("managed volume state should not be poisoned")
            .lifecycle
    }
}

impl Drop for ManagedVolumeLease {
    fn drop(&mut self) {
        self.volume.release();
    }
}

impl Drop for RouteVolumeLeaseGuard {
    fn drop(&mut self) {
        let _ = self.leases.len();
    }
}

pub(crate) async fn run_volume_gc_tick(runtime: Arc<RuntimeState>) -> Result<()> {
    let managed_paths = collect_ttl_managed_paths(&runtime.config);
    let mut handles = Vec::with_capacity(managed_paths.len());

    for managed_path in managed_paths {
        handles.push(tokio::task::spawn_blocking(move || {
            sweep_ttl_managed_path(&managed_path)
        }));
    }

    for handle in handles {
        match handle.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::warn!("volume GC worker failed: {error:#}"),
            Err(error) => tracing::warn!("volume GC blocking task failed: {error}"),
        }
    }

    Ok(())
}

pub(crate) fn collect_ttl_managed_paths(config: &IntegrityConfig) -> Vec<TtlManagedPath> {
    let mut deduped = BTreeMap::<PathBuf, Duration>::new();

    for route in &config.routes {
        for volume in &route.volumes {
            let Some(ttl_seconds) = volume.ttl_seconds else {
                continue;
            };
            let ttl = Duration::from_secs(ttl_seconds);
            let host_path = normalize_path(PathBuf::from(&volume.host_path));
            deduped
                .entry(host_path)
                .and_modify(|existing| {
                    if ttl < *existing {
                        *existing = ttl;
                    }
                })
                .or_insert(ttl);
        }
    }

    deduped
        .into_iter()
        .map(|(host_path, ttl)| TtlManagedPath { host_path, ttl })
        .collect()
}

pub(crate) fn sweep_ttl_managed_path(managed_path: &TtlManagedPath) -> Result<()> {
    let read_dir = match fs::read_dir(&managed_path.host_path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "failed to read TTL-managed path `{}`",
                    managed_path.host_path.display()
                )
            })
        }
    };

    for entry in read_dir {
        let entry = entry.with_context(|| {
            format!(
                "failed to enumerate an entry inside TTL-managed path `{}`",
                managed_path.host_path.display()
            )
        })?;
        let entry_path = entry.path();
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                continue;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to read metadata for TTL-managed entry `{}`",
                        entry_path.display()
                    )
                })
            }
        };
        let modified = match metadata.modified() {
            Ok(modified) => modified,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
                ) =>
            {
                continue;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to read modified time for TTL-managed entry `{}`",
                        entry_path.display()
                    )
                })
            }
        };

        if !ttl_entry_is_stale(modified, managed_path.ttl) {
            continue;
        }

        if let Err(error) = remove_stale_ttl_entry(&entry_path, metadata.is_dir()) {
            tracing::warn!(
                path = %entry_path.display(),
                "volume GC failed to remove stale entry gracefully: {error:#}"
            );
        }
    }

    Ok(())
}

pub(crate) fn ttl_entry_is_stale(modified: SystemTime, ttl: Duration) -> bool {
    SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|age| age >= ttl)
}

pub(crate) fn remove_stale_ttl_entry(path: &Path, is_dir: bool) -> Result<()> {
    let result = if is_dir {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };

    match result {
        Ok(()) => {
            tracing::info!(path = %path.display(), "volume GC removed stale entry");
            Ok(())
        }
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to remove stale TTL-managed entry `{}`",
                path.display()
            )
        }),
    }
}

pub(crate) fn resolve_storage_write_target(
    route: &IntegrityRoute,
    path: &str,
) -> std::result::Result<ResolvedStorageWriteTarget, String> {
    let normalized_path =
        normalize_guest_volume_path(path).map_err(|error| format!("{error:#}"))?;
    let volume = route
        .volumes
        .iter()
        .filter(|volume| guest_path_matches_volume(&normalized_path, &volume.guest_path))
        .max_by_key(|volume| volume.guest_path.len())
        .ok_or_else(|| {
            format!(
                "route `{}` cannot broker writes to `{normalized_path}` because no mounted volume matches that path",
                route.path
            )
        })?;

    let relative_path = normalized_path
        .strip_prefix(&volume.guest_path)
        .unwrap_or_default()
        .trim_start_matches('/');
    if relative_path.is_empty() {
        return Err(format!(
            "storage broker path `{normalized_path}` must target a file beneath mounted guest path `{}`",
            volume.guest_path
        ));
    }

    let volume_root = normalize_path(PathBuf::from(&volume.host_path));
    let mut host_target = volume_root.clone();
    for segment in relative_path.split('/') {
        host_target.push(segment);
    }

    Ok(ResolvedStorageWriteTarget {
        volume_root,
        guest_path: normalized_path,
        host_target,
    })
}

pub(crate) fn parse_storage_broker_host_path(
    value: &str,
    label: &str,
) -> std::result::Result<PathBuf, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("storage broker `{label}` must not be empty"));
    }

    Ok(PathBuf::from(trimmed))
}

pub(crate) fn authorize_storage_broker_write(
    config: &IntegrityConfig,
    headers: &HeaderMap,
    host_identity: &HostIdentity,
    path: &str,
) -> std::result::Result<(IntegrityRoute, ResolvedStorageWriteTarget), String> {
    let claims = host_identity.verify_header(headers)?;
    let route = config
        .sealed_route(&claims.route_path)
        .cloned()
        .ok_or_else(|| {
            forbidden_error(&format!(
                "signed caller route `{}` is not sealed in `integrity.lock`",
                claims.route_path
            ))
        })?;
    if route.role != claims.role {
        return Err(forbidden_error(&format!(
            "signed caller role mismatch for route `{}`",
            claims.route_path
        )));
    }

    let resolved =
        resolve_storage_write_target(&route, path).map_err(|error| forbidden_error(&error))?;
    Ok((route, resolved))
}

pub(crate) fn guest_path_matches_volume(path: &str, guest_path: &str) -> bool {
    path == guest_path
        || path
            .strip_prefix(guest_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

pub(crate) fn process_storage_write_request(request: &StorageBrokerWriteRequest) -> Result<()> {
    if let Some(parent) = request.host_target.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create broker parent directory for {}",
                request.host_target.display()
            )
        })?;
    }

    match request.mode {
        StorageWriteMode::Overwrite => {
            fs::write(&request.host_target, &request.body).with_context(|| {
                format!(
                    "failed to overwrite {} through storage broker",
                    request.host_target.display()
                )
            })
        }
        StorageWriteMode::Append => {
            let mut file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&request.host_target)
                .with_context(|| {
                    format!(
                        "failed to open {} for append through storage broker",
                        request.host_target.display()
                    )
                })?;
            file.write_all(&request.body).with_context(|| {
                format!(
                    "failed to append to {} through storage broker",
                    request.host_target.display()
                )
            })
        }
    }
}

pub(crate) fn emit_storage_mutation_event(
    core_store: &store::CoreStore,
    request: &StorageBrokerWriteRequest,
) -> Result<String> {
    let timestamp_unix_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .context("system clock is set before the Unix epoch")?
        .as_millis();
    let value_hash = format!("sha256:{}", hex::encode(Sha256::digest(&request.body)));
    let payload = serde_json::to_vec(&serde_json::json!({
        "event": "tachyon.data.mutation",
        "route_path": request.route_path,
        "resource": request.guest_path,
        "operation": match request.mode {
            StorageWriteMode::Overwrite => "overwrite",
            StorageWriteMode::Append => "append",
        },
        "value_hash": value_hash,
        "value_bytes": request.body.len(),
        "timestamp_unix_ms": timestamp_unix_ms,
    }))
    .context("failed to serialize CDC mutation event")?;

    core_store.append_outbox(store::CoreStoreBucket::DataMutationOutbox, &payload)
}

pub(crate) fn process_storage_snapshot_request(
    request: &StorageBrokerSnapshotRequest,
    core_store: &store::CoreStore,
) -> Result<()> {
    let _ = &request.snapshot_path;
    core_store
        .snapshot_directory(&request.volume_id, &request.source_path)
        .with_context(|| {
            format!(
                "failed to persist hibernation snapshot for volume `{}`",
                request.volume_id
            )
        })?;
    remove_path_if_exists(&request.source_path)?;
    Ok(())
}

pub(crate) fn process_storage_restore_request(
    request: &StorageBrokerRestoreRequest,
    core_store: &store::CoreStore,
) -> Result<()> {
    let restored = core_store
        .restore_directory(&request.volume_id, &request.destination_path)
        .with_context(|| {
            format!(
                "failed to restore hibernation snapshot for volume `{}`",
                request.volume_id
            )
        })?;
    if restored {
        return Ok(());
    }

    copy_directory_tree(&request.snapshot_path, &request.destination_path)
}

pub(crate) fn copy_directory_tree(source: &Path, destination: &Path) -> Result<()> {
    remove_path_if_exists(destination)?;
    fs::create_dir_all(destination).with_context(|| {
        format!(
            "failed to create destination directory `{}`",
            destination.display()
        )
    })?;

    if !source.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(source)
        .with_context(|| format!("failed to read directory `{}`", source.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry inside `{}`", source.display()))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = entry.metadata().with_context(|| {
            format!(
                "failed to read metadata for broker copy source `{}`",
                source_path.display()
            )
        })?;

        if metadata.is_dir() {
            copy_directory_tree(&source_path, &destination_path)?;
        } else {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "failed to create destination parent directory `{}`",
                        parent.display()
                    )
                })?;
            }
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "failed to copy `{}` to `{}`",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }

    Ok(())
}

pub(crate) fn remove_path_if_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to read metadata for `{}`", path.display()))?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove directory `{}`", path.display()))?;
    } else {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove file `{}`", path.display()))?;
    }

    Ok(())
}

pub(crate) fn managed_volume_key(route_path: &str, guest_path: &str) -> String {
    format!("{route_path}:{guest_path}")
}

pub(crate) fn managed_volume_id(route_path: &str, guest_path: &str) -> String {
    format!(
        "{}:{}",
        route_path.trim_matches('/').replace('/', "_"),
        guest_path.trim_matches('/').replace('/', "_")
    )
}

pub(crate) fn snapshot_path_for_volume(active_path: &Path) -> PathBuf {
    let mut snapshot = active_path.to_path_buf();
    snapshot.set_extension("snapshot");
    snapshot
}

// ── S3 volume lifecycle ────────────────────────────────────────────────────────

/// Carries the prepared temporary directory for one S3 volume invocation.
#[allow(dead_code)]
pub(crate) struct S3VolumePrep {
    /// Guest-side mount path (e.g. `/app/data`).
    pub(crate) guest_path: String,
    /// Host-side temporary directory created for this invocation.
    pub(crate) temp_path: PathBuf,
    pub(crate) readonly: bool,
    /// S3 bucket extracted from `host_path`.
    pub(crate) s3_bucket: String,
    /// S3 prefix extracted from `host_path` (may be empty).
    pub(crate) s3_prefix: String,
    /// Concurrent-write resolution mode declared on the volume.
    pub(crate) write_mode: WriteMode,
    /// Map of relative_path -> ETag captured at download time. Used by
    /// `OptimisticEtag` commits to detect concurrent modification.
    pub(crate) initial_etags: std::collections::HashMap<String, String>,
    /// Distributed lock held for the duration of the invocation when
    /// `write_mode == PessimisticLock`. Drops automatically when the prep
    /// is freed after commit + cleanup.
    pub(crate) lock_guard: Option<distributed_lock::DistributedLockGuard>,
}

impl std::fmt::Debug for S3VolumePrep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3VolumePrep")
            .field("guest_path", &self.guest_path)
            .field("temp_path", &self.temp_path)
            .field("readonly", &self.readonly)
            .field("s3_bucket", &self.s3_bucket)
            .field("s3_prefix", &self.s3_prefix)
            .field("write_mode", &self.write_mode)
            .field("initial_etags_count", &self.initial_etags.len())
            .field("has_lock", &self.lock_guard.is_some())
            .finish()
    }
}

/// Download all S3 volumes for `route` into fresh per-invocation temp dirs.
/// Returns one `S3VolumePrep` per S3 volume; empty if none are configured.
/// Must be called from an async context (before entering `spawn_blocking`).
///
/// `core_store` is used only when a volume declares
/// `consistency.write_mode = "pessimistic_lock"` — the lock is acquired
/// BEFORE the download so the invocation sees a consistent snapshot for
/// the entire prep + execute + commit lifecycle.
pub(crate) async fn prepare_s3_volumes(
    route: &IntegrityRoute,
    core_store: &std::sync::Arc<store::CoreStore>,
) -> Vec<S3VolumePrep> {
    let mut preps = Vec::new();

    for volume in route.volumes.iter().filter(|v| v.volume_type.is_s3()) {
        let (bucket, prefix) = match parse_s3_url(&volume.host_path) {
            Ok(pair) => pair,
            Err(error) => {
                tracing::warn!(
                    route = %route.path,
                    host_path = %volume.host_path,
                    "skipping S3 volume with invalid URL: {error:#}"
                );
                continue;
            }
        };

        // Pessimistic lock is acquired BEFORE download so concurrent writers
        // can't sneak modifications between our download and upload.
        let lock_guard = if volume.consistency.write_mode == WriteMode::PessimisticLock
            && !volume.readonly
        {
            let key = format!("s3-vol:{}:{}", route.path, volume.guest_path);
            let holder = leader_election::local_node_id();
            // Lease TTL of 5 minutes accommodates long-running invocations.
            // Heartbeats refresh at TTL/2 so a stuck holder eventually loses
            // the lock to another node.
            let lease = std::time::Duration::from_secs(300);
            let wait = std::time::Duration::from_secs(60);
            match distributed_lock::acquire_with_wait(core_store, &key, &holder, lease, wait).await
            {
                Ok(Some(g)) => Some(g),
                Ok(None) => {
                    tracing::warn!(
                        route = %route.path,
                        guest_path = %volume.guest_path,
                        "pessimistic_lock acquire timed out after {}s — proceeding without lock",
                        wait.as_secs()
                    );
                    None
                }
                Err(error) => {
                    tracing::warn!(
                        route = %route.path,
                        guest_path = %volume.guest_path,
                        "pessimistic_lock acquire failed: {error:#} — proceeding without lock"
                    );
                    None
                }
            }
        } else {
            None
        };

        let temp_path = match build_s3_temp_dir() {
            Ok(p) => p,
            Err(error) => {
                tracing::warn!(
                    route = %route.path,
                    guest_path = %volume.guest_path,
                    "failed to create S3 temp dir: {error:#}"
                );
                continue;
            }
        };

        #[cfg_attr(not(feature = "s3-persistence"), allow(unused_mut))]
        let mut initial_etags = std::collections::HashMap::new();
        #[cfg(feature = "s3-persistence")]
        {
            match download_s3_prefix_to_dir(&bucket, &prefix, &temp_path).await {
                Ok(etags) => initial_etags = etags,
                Err(error) => {
                    tracing::warn!(
                        route = %route.path,
                        guest_path = %volume.guest_path,
                        "S3 volume download failed, guest will see empty dir: {error:#}"
                    );
                }
            }
        }
        #[cfg(not(feature = "s3-persistence"))]
        {
            tracing::warn!(
                route = %route.path,
                guest_path = %volume.guest_path,
                bucket = %bucket,
                "S3 volume configured but binary was compiled without s3-persistence feature — guest will see empty dir"
            );
        }

        preps.push(S3VolumePrep {
            guest_path: volume.guest_path.clone(),
            temp_path,
            readonly: volume.readonly,
            s3_bucket: bucket,
            s3_prefix: prefix,
            write_mode: volume.consistency.write_mode,
            initial_etags,
            lock_guard,
        });
    }

    preps
}

/// Upload modified files back to S3 for all read-write volumes.
/// Dispatches on the volume's `write_mode` to apply the appropriate
/// concurrency-resolution strategy (LWW unconditional / optimistic ETag /
/// pessimistic lock). Called after successful guest execution.
pub(crate) async fn commit_s3_volumes(preps: &[S3VolumePrep]) {
    #[cfg(feature = "s3-persistence")]
    for prep in preps.iter().filter(|p| !p.readonly) {
        let result = match prep.write_mode {
            WriteMode::LastWriteWins | WriteMode::PessimisticLock => {
                // PessimisticLock holds the lock around the whole invocation
                // upstream, so the commit itself is a simple unconditional upload.
                upload_dir_to_s3_prefix(&prep.temp_path, &prep.s3_bucket, &prep.s3_prefix).await
            }
            WriteMode::OptimisticEtag => {
                upload_dir_to_s3_prefix_with_etag(
                    &prep.temp_path,
                    &prep.s3_bucket,
                    &prep.s3_prefix,
                    &prep.initial_etags,
                )
                .await
            }
            // None is rejected at schema validation when !readonly; treat as no-op defensively.
            WriteMode::None => Ok(()),
        };
        if let Err(error) = result {
            tracing::warn!(
                guest_path = %prep.guest_path,
                bucket = %prep.s3_bucket,
                prefix = %prep.s3_prefix,
                write_mode = ?prep.write_mode,
                "S3 volume commit failed: {error:#}"
            );
        }
    }
    #[cfg(not(feature = "s3-persistence"))]
    let _ = preps;
}

/// Delete all temporary directories created by `prepare_s3_volumes`.
/// Called unconditionally after execution (success or failure).
pub(crate) fn cleanup_s3_volume_dirs(preps: &[S3VolumePrep]) {
    for prep in preps {
        if let Err(error) = std::fs::remove_dir_all(&prep.temp_path) {
            tracing::debug!(
                path = %prep.temp_path.display(),
                "failed to remove S3 temp dir: {error}"
            );
        }
    }
}

fn build_s3_temp_dir() -> std::io::Result<PathBuf> {
    let uuid = uuid_v4_hex();
    let dir = std::env::temp_dir().join(format!("tachyon-s3-vol-{uuid}"));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn uuid_v4_hex() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    // Simple unique ID without pulling in the uuid crate.
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    // Mix thread id and timestamp nanos for a reasonably unique suffix.
    let tid = format!("{:?}", std::thread::current().id());
    let h = format!("{:08x}{:08x}", t, tid.len().wrapping_mul(0x9e3779b9));
    h
}

#[cfg(feature = "s3-persistence")]
/// Download the S3 prefix into `dest` and return a map of
/// `relative_path -> e_tag` captured at download time. The map lets the
/// commit phase use `OptimisticEtag` conditional PUTs.
async fn download_s3_prefix_to_dir(
    bucket: &str,
    prefix: &str,
    dest: &Path,
) -> anyhow::Result<std::collections::HashMap<String, String>> {
    use futures::StreamExt as _;
    use object_store::{path::Path as OsPath, ObjectStore, ObjectStoreExt};

    let store = build_s3_store(bucket)?;
    let prefix_path = if prefix.is_empty() {
        None
    } else {
        Some(OsPath::parse(prefix).map_err(|e| anyhow::anyhow!("{e}"))?)
    };

    let mut etags = std::collections::HashMap::new();
    let mut list = store.list(prefix_path.as_ref());
    while let Some(entry) = list.next().await {
        let meta = entry?;
        let key_str = meta.location.to_string();
        let rel = if prefix.is_empty() {
            key_str.as_str()
        } else {
            key_str
                .strip_prefix(&format!("{}/", prefix.trim_end_matches('/')))
                .unwrap_or(key_str.as_str())
        };

        let local_path = dest.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = local_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let result = store.get(&meta.location).await?;
        if let Some(etag) = &result.meta.e_tag {
            etags.insert(rel.to_owned(), etag.clone());
        }
        let bytes = result.bytes().await?;
        tokio::fs::write(&local_path, &bytes).await?;
    }
    Ok(etags)
}

#[cfg(feature = "s3-persistence")]
async fn upload_dir_to_s3_prefix(dir: &Path, bucket: &str, prefix: &str) -> anyhow::Result<()> {
    use object_store::{path::Path as OsPath, ObjectStoreExt};

    let store = build_s3_store(bucket)?;
    let mut stack = vec![dir.to_path_buf()];

    while let Some(entry_path) = stack.pop() {
        let metadata = match tokio::fs::metadata(&entry_path).await {
            Ok(m) => m,
            Err(_) => continue,
        };
        if metadata.is_dir() {
            let mut read = tokio::fs::read_dir(&entry_path).await?;
            while let Some(child) = read.next_entry().await? {
                stack.push(child.path());
            }
        } else {
            let rel = entry_path
                .strip_prefix(dir)
                .unwrap_or(&entry_path)
                .to_string_lossy()
                .replace('\\', "/");
            let key_str = if prefix.is_empty() {
                rel
            } else {
                format!("{}/{}", prefix.trim_end_matches('/'), rel)
            };
            let key = OsPath::parse(&key_str).map_err(|e| anyhow::anyhow!("{e}"))?;
            let bytes = tokio::fs::read(&entry_path).await?;
            store.put(&key, bytes.into()).await?;
        }
    }
    Ok(())
}

/// Like `upload_dir_to_s3_prefix` but uses a conditional PUT (`If-Match: <etag>`)
/// for every file whose ETag was captured at download time. Aborts on the first
/// 412 Precondition Failed so the caller surfaces the conflict to the operator.
#[cfg(feature = "s3-persistence")]
async fn upload_dir_to_s3_prefix_with_etag(
    dir: &Path,
    bucket: &str,
    prefix: &str,
    initial_etags: &std::collections::HashMap<String, String>,
) -> anyhow::Result<()> {
    use object_store::{path::Path as OsPath, ObjectStore, PutMode, PutOptions};

    let store = build_s3_store(bucket)?;
    let mut stack = vec![dir.to_path_buf()];

    while let Some(entry_path) = stack.pop() {
        let metadata = match tokio::fs::metadata(&entry_path).await {
            Ok(m) => m,
            Err(_) => continue,
        };
        if metadata.is_dir() {
            let mut read = tokio::fs::read_dir(&entry_path).await?;
            while let Some(child) = read.next_entry().await? {
                stack.push(child.path());
            }
        } else {
            let rel = entry_path
                .strip_prefix(dir)
                .unwrap_or(&entry_path)
                .to_string_lossy()
                .replace('\\', "/");
            let key_str = if prefix.is_empty() {
                rel.clone()
            } else {
                format!("{}/{}", prefix.trim_end_matches('/'), rel)
            };
            let key = OsPath::parse(&key_str).map_err(|e| anyhow::anyhow!("{e}"))?;
            let bytes = tokio::fs::read(&entry_path).await?;
            let mode = match initial_etags.get(&rel) {
                Some(etag) => PutMode::Update(object_store::UpdateVersion {
                    e_tag: Some(etag.clone()),
                    version: None,
                }),
                None => PutMode::Create,
            };
            let opts = PutOptions {
                mode,
                ..Default::default()
            };
            store
                .put_opts(&key, bytes.into(), opts)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "optimistic_etag commit failed for {key_str}: {e} \
                         (another writer modified the object since download)"
                    )
                })?;
        }
    }
    Ok(())
}

#[cfg(feature = "s3-persistence")]
pub(crate) fn build_s3_store(bucket: &str) -> anyhow::Result<impl object_store::ObjectStore> {
    use object_store::aws::AmazonS3Builder;
    let mut builder = AmazonS3Builder::new()
        .with_bucket_name(bucket)
        .with_access_key_id(std::env::var("TACHYON_S3_ACCESS_KEY_ID").unwrap_or_default())
        .with_secret_access_key(std::env::var("TACHYON_S3_SECRET_ACCESS_KEY").unwrap_or_default())
        .with_region(std::env::var("TACHYON_S3_REGION").unwrap_or_else(|_| "us-east-1".to_owned()))
        .with_allow_http(true);
    if let Ok(endpoint) = std::env::var("TACHYON_S3_ENDPOINT") {
        builder = builder.with_endpoint(endpoint);
    }
    builder.build().map_err(|e| anyhow::anyhow!("{e}"))
}
