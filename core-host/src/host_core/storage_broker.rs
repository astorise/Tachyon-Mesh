impl StorageBrokerManager {
    fn new(core_store: Arc<store::CoreStore>) -> Self {
        Self {
            core_store,
            queues: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    #[cfg(test)]
    fn enqueue_write_for_route(
        &self,
        route: &IntegrityRoute,
        path: &str,
        mode: StorageWriteMode,
        body: Vec<u8>,
    ) -> std::result::Result<(), String> {
        let resolved = resolve_storage_write_target(route, path)?;
        self.enqueue_write_target(
            route.path.clone(),
            route.sync_to_cloud,
            resolved,
            mode,
            body,
        )
    }

    fn enqueue_write_target(
        &self,
        route_path: String,
        sync_to_cloud: bool,
        resolved: ResolvedStorageWriteTarget,
        mode: StorageWriteMode,
        body: Vec<u8>,
    ) -> std::result::Result<(), String> {
        let queue = self.queue_for_volume(&resolved.volume_root);
        queue.enqueue(StorageBrokerOperation::Write(StorageBrokerWriteRequest {
            route_path,
            guest_path: resolved.guest_path,
            host_target: resolved.host_target,
            mode,
            body,
            sync_to_cloud,
        }))
    }

    fn enqueue_snapshot(
        &self,
        volume_id: String,
        volume_root: &Path,
        source_path: &Path,
        snapshot_path: &Path,
    ) -> std::result::Result<tokio::sync::oneshot::Receiver<std::result::Result<(), String>>, String>
    {
        let queue = self.queue_for_volume(volume_root);
        let (completion, receiver) = tokio::sync::oneshot::channel();
        queue.enqueue(StorageBrokerOperation::Snapshot(
            StorageBrokerSnapshotRequest {
                volume_id,
                source_path: source_path.to_path_buf(),
                snapshot_path: snapshot_path.to_path_buf(),
                completion,
            },
        ))?;
        Ok(receiver)
    }

    fn enqueue_restore(
        &self,
        volume_id: String,
        volume_root: &Path,
        snapshot_path: &Path,
        destination_path: &Path,
    ) -> std::result::Result<tokio::sync::oneshot::Receiver<std::result::Result<(), String>>, String>
    {
        let queue = self.queue_for_volume(volume_root);
        let (completion, receiver) = tokio::sync::oneshot::channel();
        queue.enqueue(StorageBrokerOperation::Restore(
            StorageBrokerRestoreRequest {
                volume_id,
                snapshot_path: snapshot_path.to_path_buf(),
                destination_path: destination_path.to_path_buf(),
                completion,
            },
        ))?;
        Ok(receiver)
    }

    fn queue_for_volume(&self, volume_root: &Path) -> Arc<StorageVolumeQueue> {
        let key = normalize_path(volume_root.to_path_buf());
        let mut queues = self
            .queues
            .lock()
            .expect("storage broker queues should not be poisoned");
        Arc::clone(
            queues
                .entry(key.clone())
                .or_insert_with(|| StorageVolumeQueue::new(key, Arc::clone(&self.core_store))),
        )
    }

    #[cfg(test)]
    fn wait_for_volume_idle(&self, volume_root: &Path, timeout: Duration) -> bool {
        self.queue_for_volume(volume_root).wait_for_idle(timeout)
    }
}

impl Default for StorageBrokerManager {
    fn default() -> Self {
        let path = std::env::temp_dir().join(format!("tachyon-store-{}.db", Uuid::new_v4()));
        let core_store =
            store::CoreStore::open(&path).expect("default storage broker core store should open");
        Self::new(Arc::new(core_store))
    }
}

impl StorageVolumeQueue {
    fn new(volume_root: PathBuf, core_store: Arc<store::CoreStore>) -> Arc<Self> {
        let (sender, receiver) = std::sync::mpsc::channel::<StorageBrokerOperation>();
        let queue = Arc::new(Self {
            volume_root,
            core_store,
            sender,
            state: Mutex::new(StorageVolumeQueueState::default()),
            idle: Condvar::new(),
        });
        let worker = Arc::clone(&queue);
        std::thread::spawn(move || worker.run(receiver));
        queue
    }

    fn enqueue(&self, operation: StorageBrokerOperation) -> std::result::Result<(), String> {
        self.state
            .lock()
            .expect("storage broker queue state should not be poisoned")
            .pending += 1;
        if self.sender.send(operation).is_ok() {
            return Ok(());
        }

        let mut state = self
            .state
            .lock()
            .expect("storage broker queue state should not be poisoned");
        state.pending = state.pending.saturating_sub(1);
        self.idle.notify_all();
        Err(format!(
            "storage broker queue for `{}` is not available",
            self.volume_root.display()
        ))
    }

    fn run(self: Arc<Self>, receiver: std::sync::mpsc::Receiver<StorageBrokerOperation>) {
        while let Ok(operation) = receiver.recv() {
            match operation {
                StorageBrokerOperation::Write(request) => {
                    if let Err(error) = process_storage_write_request(&request) {
                        tracing::warn!(
                            route = %request.route_path,
                            guest_path = %request.guest_path,
                            host_target = %request.host_target.display(),
                            "storage broker write failed: {error}"
                        );
                    } else if request.sync_to_cloud {
                        if let Err(error) = emit_storage_mutation_event(&self.core_store, &request)
                        {
                            tracing::warn!(
                                route = %request.route_path,
                                guest_path = %request.guest_path,
                                host_target = %request.host_target.display(),
                                "storage broker CDC event emit failed: {error:#}"
                            );
                        }
                    }
                }
                StorageBrokerOperation::Snapshot(request) => {
                    let result = process_storage_snapshot_request(&request, &self.core_store)
                        .map_err(|error| format!("{error:#}"));
                    if let Err(error) = &result {
                        tracing::warn!(
                            volume_id = %request.volume_id,
                            snapshot_path = %request.snapshot_path.display(),
                            "storage broker snapshot failed: {error}"
                        );
                    }
                    let _ = request.completion.send(result);
                }
                StorageBrokerOperation::Restore(request) => {
                    let result = process_storage_restore_request(&request, &self.core_store)
                        .map_err(|error| format!("{error:#}"));
                    if let Err(error) = &result {
                        tracing::warn!(
                            volume_id = %request.volume_id,
                            snapshot_path = %request.snapshot_path.display(),
                            destination_path = %request.destination_path.display(),
                            "storage broker restore failed: {error}"
                        );
                    }
                    let _ = request.completion.send(result);
                }
            }

            let mut state = self
                .state
                .lock()
                .expect("storage broker queue state should not be poisoned");
            state.pending = state.pending.saturating_sub(1);
            self.idle.notify_all();
        }
    }

    #[cfg(test)]
    fn wait_for_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .state
            .lock()
            .expect("storage broker queue state should not be poisoned");

        while state.pending > 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return false;
            }

            let (next_state, result) = self
                .idle
                .wait_timeout(state, remaining)
                .expect("storage broker queue state should not be poisoned");
            state = next_state;
            if result.timed_out() && state.pending > 0 {
                return false;
            }
        }

        true
    }
}

impl BufferedRequestManager {
    fn new(disk_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&disk_dir).with_context(|| {
            format!(
                "failed to initialize buffered request spool directory `{}`",
                disk_dir.display()
            )
        })?;
        Ok(Self {
            disk_dir,
            ram_capacity: BUFFER_RAM_REQUEST_CAPACITY,
            total_capacity: BUFFER_TOTAL_REQUEST_CAPACITY,
            state: Arc::new(Mutex::new(BufferedRequestState {
                next_id: 0,
                ram_queue: VecDeque::new(),
                disk_queue: VecDeque::new(),
            })),
            notify: Arc::new(Notify::new()),
        })
    }

    fn enqueue(
        &self,
        request: BufferedRouteRequest,
    ) -> std::result::Result<(oneshot::Receiver<BufferedRouteResult>, BufferedRequestTier), String>
    {
        let (completion, receiver) = oneshot::channel();
        let mut state = self
            .state
            .lock()
            .expect("buffered request state should not be poisoned");
        let total_queued = state.ram_queue.len() + state.disk_queue.len();
        if total_queued >= self.total_capacity {
            return Err("buffer manager is full".to_owned());
        }

        let id = format!("buffered-{}", state.next_id);
        state.next_id = state.next_id.saturating_add(1);
        if state.ram_queue.len() < self.ram_capacity {
            state.ram_queue.push_back(BufferedMemoryRequest {
                id,
                request,
                completion,
            });
            drop(state);
            self.notify.notify_one();
            Ok((receiver, BufferedRequestTier::Ram))
        } else {
            let path = self.disk_dir.join(format!("{id}.json"));
            let payload = serde_json::to_vec(&request)
                .map_err(|error| format!("failed to serialize buffered request: {error}"))?;
            fs::write(&path, payload).map_err(|error| {
                format!(
                    "failed to persist buffered request spool file `{}`: {error}",
                    path.display()
                )
            })?;
            state.disk_queue.push_back(BufferedDiskRequest {
                id,
                path,
                completion,
            });
            drop(state);
            self.notify.notify_one();
            Ok((receiver, BufferedRequestTier::Disk))
        }
    }

    fn pop_next(&self) -> std::result::Result<Option<BufferedQueueItem>, String> {
        let queued = {
            let mut state = self
                .state
                .lock()
                .expect("buffered request state should not be poisoned");
            if let Some(request) = state.ram_queue.pop_front() {
                return Ok(Some(BufferedQueueItem {
                    id: request.id,
                    request: request.request,
                    completion: request.completion,
                    disk_path: None,
                }));
            }
            state.disk_queue.pop_front()
        };

        let Some(request) = queued else {
            return Ok(None);
        };
        let payload = fs::read(&request.path).map_err(|error| {
            format!(
                "failed to read buffered request spool file `{}`: {error}",
                request.path.display()
            )
        })?;
        let buffered = serde_json::from_slice(&payload)
            .map_err(|error| format!("failed to deserialize buffered request: {error}"))?;
        Ok(Some(BufferedQueueItem {
            id: request.id,
            request: buffered,
            completion: request.completion,
            disk_path: Some(request.path),
        }))
    }

    fn requeue_front(&self, item: BufferedQueueItem) -> std::result::Result<(), String> {
        let mut state = self
            .state
            .lock()
            .expect("buffered request state should not be poisoned");
        match item.disk_path {
            Some(path) => state.disk_queue.push_front(BufferedDiskRequest {
                id: item.id,
                path,
                completion: item.completion,
            }),
            None => state.ram_queue.push_front(BufferedMemoryRequest {
                id: item.id,
                request: item.request,
                completion: item.completion,
            }),
        }
        drop(state);
        self.notify.notify_one();
        Ok(())
    }

    fn complete(&self, item: BufferedQueueItem, result: BufferedRouteResult) {
        if let Some(path) = &item.disk_path {
            let _ = fs::remove_file(path);
        }
        let _ = item.completion.send(result);
    }

    fn pending_count(&self) -> usize {
        let state = self
            .state
            .lock()
            .expect("buffered request state should not be poisoned");
        state.ram_queue.len() + state.disk_queue.len()
    }

    #[cfg(test)]
    fn disk_spill_count(&self) -> usize {
        self.state
            .lock()
            .expect("buffered request state should not be poisoned")
            .disk_queue
            .len()
    }
}
