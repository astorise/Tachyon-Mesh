impl VolumeManager {
    async fn acquire_route_volumes(
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

    fn managed_volume(
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
    fn managed_volume_for_route(
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
    fn new(
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

    async fn acquire(self: &Arc<Self>) -> std::result::Result<ManagedVolumeLease, String> {
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

    fn release(self: &Arc<Self>) {
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

    fn schedule_hibernation(self: &Arc<Self>, generation: u64) {
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

    fn finish_restore(&self, lifecycle: ManagedVolumeLifecycle) {
        let mut state = self
            .state
            .lock()
            .expect("managed volume state should not be poisoned");
        state.lifecycle = lifecycle;
        state.generation = state.generation.saturating_add(1);
        self.notify.notify_waiters();
    }

    #[cfg(test)]
    fn lifecycle(&self) -> ManagedVolumeLifecycle {
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

async fn run_volume_gc_tick(runtime: Arc<RuntimeState>) -> Result<()> {
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

fn collect_ttl_managed_paths(config: &IntegrityConfig) -> Vec<TtlManagedPath> {
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

fn sweep_ttl_managed_path(managed_path: &TtlManagedPath) -> Result<()> {
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

fn ttl_entry_is_stale(modified: SystemTime, ttl: Duration) -> bool {
    SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|age| age >= ttl)
}

fn remove_stale_ttl_entry(path: &Path, is_dir: bool) -> Result<()> {
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

fn resolve_storage_write_target(
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

fn parse_storage_broker_host_path(
    value: &str,
    label: &str,
) -> std::result::Result<PathBuf, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("storage broker `{label}` must not be empty"));
    }

    Ok(PathBuf::from(trimmed))
}

fn authorize_storage_broker_write(
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

fn guest_path_matches_volume(path: &str, guest_path: &str) -> bool {
    path == guest_path
        || path
            .strip_prefix(guest_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn process_storage_write_request(request: &StorageBrokerWriteRequest) -> Result<()> {
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

fn emit_storage_mutation_event(
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

fn process_storage_snapshot_request(
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

fn process_storage_restore_request(
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

fn copy_directory_tree(source: &Path, destination: &Path) -> Result<()> {
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

fn remove_path_if_exists(path: &Path) -> Result<()> {
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

fn managed_volume_key(route_path: &str, guest_path: &str) -> String {
    format!("{route_path}:{guest_path}")
}

fn managed_volume_id(route_path: &str, guest_path: &str) -> String {
    format!(
        "{}:{}",
        route_path.trim_matches('/').replace('/', "_"),
        guest_path.trim_matches('/').replace('/', "_")
    )
}

fn snapshot_path_for_volume(active_path: &Path) -> PathBuf {
    let mut snapshot = active_path.to_path_buf();
    snapshot.set_extension("snapshot");
    snapshot
}
