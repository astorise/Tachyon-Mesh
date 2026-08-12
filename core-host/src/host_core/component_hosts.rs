use super::*;

impl LegacyHostState {
    pub(crate) fn new(
        wasi: WasiP1Ctx,
        max_memory_bytes: usize,
        #[cfg(feature = "ai-inference")] ai_runtime: Arc<ai_inference::AiInferenceRuntime>,
    ) -> Self {
        Self {
            wasi,
            #[cfg(feature = "ai-inference")]
            wasi_nn: build_wasi_nn_ctx(ai_runtime.as_ref()),
            limits: GuestResourceLimiter::new(max_memory_bytes),
        }
    }
}

#[cfg(feature = "ai-inference")]
pub(crate) fn build_wasi_nn_ctx(runtime: &ai_inference::AiInferenceRuntime) -> WasiNnCtx {
    runtime.build_wasi_nn_ctx()
}

impl SecretsVault {
    pub(crate) fn load() -> Self {
        #[cfg(feature = "secrets-vault")]
        {
            let entries = HashMap::from([("DB_PASS".to_owned(), "super_secret_123".to_owned())]);
            Self {
                entries: Arc::new(entries),
            }
        }

        #[cfg(not(feature = "secrets-vault"))]
        {
            Self::default()
        }
    }
}

impl SecretAccess {
    pub(crate) fn from_route(route: &IntegrityRoute, _vault: &SecretsVault) -> Self {
        Self {
            allowed_secrets: route.allowed_secrets.iter().cloned().collect(),
            #[cfg(feature = "secrets-vault")]
            entries: Arc::clone(&_vault.entries),
        }
    }

    pub(crate) fn get_secret(
        &self,
        name: &str,
    ) -> std::result::Result<String, SecretAccessErrorKind> {
        #[cfg(not(feature = "secrets-vault"))]
        {
            let _ = name;
            Err(SecretAccessErrorKind::VaultDisabled)
        }

        #[cfg(feature = "secrets-vault")]
        {
            if !self.allowed_secrets.contains(name) {
                return Err(SecretAccessErrorKind::PermissionDenied);
            }

            self.entries
                .get(name)
                .cloned()
                .ok_or(SecretAccessErrorKind::NotFound)
        }
    }
}

impl ComponentHostState {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        route: &IntegrityRoute,
        runtime_config: IntegrityConfig,
        max_memory_bytes: usize,
        telemetry: TelemetryHandle,
        secrets: SecretAccess,
        request_headers: HeaderMap,
        host_identity: Arc<HostIdentity>,
        storage_broker: Arc<StorageBrokerManager>,
        concurrency_limits: Arc<HashMap<String, Arc<RouteExecutionControl>>>,
        propagated_headers: Vec<PropagatedHeader>,
        local_mesh_dispatch: Option<LocalMeshDispatchContext>,
        s3_preps: &[crate::host_core::volumes::S3VolumePrep],
    ) -> std::result::Result<Self, ExecutionError> {
        let scopes = {
            use crate::host_core::scoping::DeploymentScopes;
            match &route.scopes {
                Some(value) => {
                    // Already validated at manifest submission; errors here are
                    // unexpected but we fall back to allow-all rather than crash.
                    match DeploymentScopes::from_manifest(value) {
                        Ok(s) => Arc::new(s),
                        Err(e) => {
                            tracing::warn!(
                                route = %route.path,
                                error = %e,
                                "scope manifest re-parse failed at instantiation; falling back to allow-all"
                            );
                            crate::host_core::scoping::record_scope_allow_all_prometheus(
                                &route.path,
                            );
                            Arc::new(DeploymentScopes::allow_all())
                        }
                    }
                }
                None => {
                    tracing::warn!(
                        route = %route.path,
                        "route has no `scopes` block — defaulting to allow-all (consider adding explicit scopes)"
                    );
                    crate::host_core::scoping::record_scope_allow_all_prometheus(&route.path);
                    Arc::new(DeploymentScopes::allow_all())
                }
            }
        };
        Ok(Self {
            ctx: build_component_wasi_ctx(route, host_identity.as_ref(), s3_preps)?,
            table: ResourceTable::new(),
            limits: GuestResourceLimiter::new(max_memory_bytes),
            secrets,
            scopes,
            scope_denials: Arc::new(crate::host_core::scoping::ScopeDenialCounters::default()),
            runtime_config,
            request_headers,
            host_identity,
            storage_broker,
            bridge_manager: Arc::new(BridgeManager::default()),
            telemetry,
            concurrency_limits,
            propagated_headers,
            route_overrides: Arc::new(ArcSwap::from_pointee(HashMap::new())),
            peer_capabilities: Arc::new(Mutex::new(HashMap::new())),
            host_capabilities: Capabilities::detect(),
            host_load: Arc::new(HostLoadCounters::default()),
            outbound_http_client: blocking_outbound_http_client(),
            local_mesh_dispatch,
            route_path: route.path.clone(),
            route_role: route.role,
            #[cfg(feature = "ai-inference")]
            ai_runtime: None,
            #[cfg(feature = "ai-inference")]
            allowed_model_aliases: route
                .models
                .iter()
                .map(|binding| binding.alias.clone())
                .collect(),
            #[cfg(feature = "ai-inference")]
            adapter_id: route.adapter_id.clone(),
            #[cfg(feature = "ai-inference")]
            accelerator_models: HashMap::new(),
            #[cfg(feature = "ai-inference")]
            next_accelerator_model_id: 1,
            #[cfg(feature = "ai-inference")]
            streaming_consumer_alive: None,
            streaming_body: None,
        })
    }

    pub(crate) fn pending_queue_size(&self, route_path: &str) -> u32 {
        self.concurrency_limits
            .get(&normalize_route_path(route_path))
            .map(|control| control.keda_pending_queue_size())
            .unwrap_or_default()
    }

    pub(crate) fn vector_tenant_id(&self) -> String {
        self.request_headers
            .get("x-tachyon-tenant")
            .or_else(|| self.request_headers.get("x-tenant-id"))
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(&self.route_path)
            .to_owned()
    }

    pub(crate) fn hot_model_aliases(&self) -> Vec<String> {
        #[cfg(feature = "ai-inference")]
        {
            self.ai_runtime
                .as_ref()
                .map(|runtime| runtime.loaded_model_aliases())
                .unwrap_or_default()
        }

        #[cfg(not(feature = "ai-inference"))]
        {
            Vec::new()
        }
    }

    pub(crate) fn accelerator_queue_loads(&self) -> AcceleratorQueueLoads {
        #[cfg(feature = "ai-inference")]
        {
            let Some(runtime) = self.ai_runtime.as_ref() else {
                return AcceleratorQueueLoads::default();
            };
            let cpu = runtime.queue_tier_snapshot(ai_inference::AcceleratorKind::Cpu);
            let gpu = runtime.queue_tier_snapshot(ai_inference::AcceleratorKind::Gpu);
            let npu = runtime.queue_tier_snapshot(ai_inference::AcceleratorKind::Npu);
            let tpu = runtime.queue_tier_snapshot(ai_inference::AcceleratorKind::Tpu);
            let network = runtime.queue_tier_snapshot(ai_inference::AcceleratorKind::Network);
            AcceleratorQueueLoads {
                cpu_rt_load: cpu.realtime,
                cpu_standard_load: cpu.standard,
                cpu_batch_load: cpu.batch,
                gpu_rt_load: gpu.realtime,
                gpu_standard_load: gpu.standard,
                gpu_batch_load: gpu.batch,
                npu_rt_load: npu.realtime,
                npu_standard_load: npu.standard,
                npu_batch_load: npu.batch,
                tpu_rt_load: tpu.realtime,
                tpu_standard_load: tpu.standard,
                tpu_batch_load: tpu.batch,
                network_rt_load: network.realtime,
                network_standard_load: network.standard,
                network_batch_load: network.batch,
            }
        }

        #[cfg(not(feature = "ai-inference"))]
        {
            AcceleratorQueueLoads::default()
        }
    }

    pub(crate) fn handle_bridge_create(
        &self,
        config: BridgeConfig,
    ) -> std::result::Result<BridgeAllocation, String> {
        if self.route_role == RouteRole::System && self.route_path == SYSTEM_BRIDGE_ROUTE {
            let mut allocation = self.bridge_manager.create_relay(config)?;
            allocation.ip = effective_advertise_ip(&self.runtime_config);
            return Ok(allocation);
        }

        let url = rewrite_outbound_http_url("http://mesh/system/bridge", &self.runtime_config);
        let response = self
            .outbound_http_client
            .post(&url)
            .header("content-type", "application/json")
            .body(
                serde_json::to_vec(&config)
                    .map_err(|error| format!("failed to encode bridge config: {error}"))?,
            )
            .send()
            .map_err(|error| format!("failed to call system bridge controller: {error}"))?;
        let status = response.status();
        let body = response
            .bytes()
            .map_err(|error| format!("failed to read bridge controller response: {error}"))?;
        if !status.is_success() {
            return Err(format!(
                "system bridge controller returned HTTP {}: {}",
                status,
                String::from_utf8_lossy(&body)
            ));
        }
        serde_json::from_slice(&body)
            .map_err(|error| format!("failed to decode bridge allocation response: {error}"))
    }

    pub(crate) fn handle_bridge_destroy(&self, bridge_id: &str) -> std::result::Result<(), String> {
        if self.route_role == RouteRole::System && self.route_path == SYSTEM_BRIDGE_ROUTE {
            return self.bridge_manager.destroy_relay(bridge_id);
        }

        let url = rewrite_outbound_http_url("http://mesh/system/bridge", &self.runtime_config);
        let response = self
            .outbound_http_client
            .delete(&url)
            .header("content-type", "application/json")
            .body(
                serde_json::to_vec(&serde_json::json!({ "bridge_id": bridge_id })).map_err(
                    |error| format!("failed to encode bridge teardown request: {error}"),
                )?,
            )
            .send()
            .map_err(|error| format!("failed to call system bridge teardown: {error}"))?;
        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response
                .bytes()
                .map_err(|error| format!("failed to read bridge teardown response: {error}"))?;
            Err(format!(
                "system bridge teardown returned HTTP {}: {}",
                status,
                String::from_utf8_lossy(&body)
            ))
        }
    }

    #[cfg(feature = "ai-inference")]
    pub(crate) fn load_accelerator_model(
        &mut self,
        accelerator: ai_inference::AcceleratorKind,
        alias: String,
    ) -> std::result::Result<u32, String> {
        if !self.allowed_model_aliases.contains(&alias) {
            return Err(format!(
                "model alias `{alias}` is not sealed for this route"
            ));
        }
        self.ai_runtime
            .as_ref()
            .ok_or_else(|| "AI inference runtime is unavailable for this component".to_owned())?
            .load_component_model(&alias, accelerator)?;
        let model_id = self.next_accelerator_model_id;
        self.next_accelerator_model_id = self.next_accelerator_model_id.saturating_add(1);
        self.accelerator_models
            .insert(model_id, LoadedAcceleratorModel { alias, accelerator });
        Ok(model_id)
    }

    #[cfg(feature = "ai-inference")]
    pub(crate) fn compute_accelerator_prompt(
        &self,
        expected_accelerator: ai_inference::AcceleratorKind,
        model_id: u32,
        prompt: String,
    ) -> std::result::Result<ai_inference::ComponentGeneration, ai_inference::GenerationError> {
        let loaded = self.resolve_accelerator_model(expected_accelerator, model_id)?;
        self.ai_runtime
            .as_ref()
            .ok_or_else(|| {
                ai_inference::GenerationError::local(
                    "AI inference runtime is unavailable for this component",
                )
            })?
            .compute_component_prompt_with_adapter(
                &loaded.alias,
                &prompt,
                self.adapter_id.as_deref(),
            )
    }

    /// Resolve a guest-held model handle to the alias it was opened for.
    ///
    /// Shared by every accelerator entry point so the handle check — the gate
    /// that stops a component reaching a model it never loaded, or reaching a
    /// CPU-bound alias through the GPU interface — cannot drift between them.
    #[cfg(feature = "ai-inference")]
    fn resolve_accelerator_model(
        &self,
        expected_accelerator: ai_inference::AcceleratorKind,
        model_id: u32,
    ) -> std::result::Result<&LoadedAcceleratorModel, ai_inference::GenerationError> {
        let loaded = self.accelerator_models.get(&model_id).ok_or_else(|| {
            ai_inference::GenerationError::local(format!(
                "accelerator model handle `{model_id}` is unknown"
            ))
        })?;
        if loaded.accelerator != expected_accelerator {
            return Err(ai_inference::GenerationError::local(format!(
                "accelerator model handle `{model_id}` was loaded for `{}` not `{}`",
                loaded.accelerator.as_str(),
                expected_accelerator.as_str()
            )));
        }
        Ok(loaded)
    }

    #[cfg(feature = "ai-inference")]
    pub(crate) fn embed_accelerator_input(
        &self,
        expected_accelerator: ai_inference::AcceleratorKind,
        model_id: u32,
        input: String,
    ) -> std::result::Result<Vec<f32>, ai_inference::GenerationError> {
        let loaded = self.resolve_accelerator_model(expected_accelerator, model_id)?;
        self.ai_runtime
            .as_ref()
            .ok_or_else(|| {
                ai_inference::GenerationError::local(
                    "AI inference runtime is unavailable for this component",
                )
            })?
            .embed_component_input(&loaded.alias, &input)
    }

    /// Begin a streaming generation: resolve the model handle (the same scope
    /// and accelerator checks as `compute_accelerator_prompt`), then run the
    /// decode on a dedicated thread that pushes each decoded text fragment into
    /// a channel. The returned receiver is drained by the `token-stream`
    /// resource's `next` calls, giving the guest real time-to-first-token.
    #[cfg(feature = "ai-inference")]
    pub(crate) fn stream_accelerator_prompt(
        &self,
        expected_accelerator: ai_inference::AcceleratorKind,
        model_id: u32,
        prompt: String,
    ) -> std::result::Result<StreamedGeneration, ai_inference::GenerationError> {
        let loaded = self.resolve_accelerator_model(expected_accelerator, model_id)?;
        let alias = loaded.alias.clone();
        let adapter_id = self.adapter_id.clone();
        let ai_runtime = Arc::clone(self.ai_runtime.as_ref().ok_or_else(|| {
            ai_inference::GenerationError::local(
                "AI inference runtime is unavailable for this component",
            )
        })?);
        // Bounded, so the generation thread advances only as fast as the guest
        // drains it. An unbounded channel let a fast upstream enqueue a whole
        // stream for a slow-but-connected client: the per-stream cap is 64 MiB
        // and the node admits 32 concurrent streams, so queued payloads could
        // reach gigabytes while every cancellation check stayed quiet — nobody
        // had disconnected, they were simply behind.
        //
        // `sync_channel` also makes back-pressure the *same* signal as
        // cancellation: `send` blocks while the consumer is merely slow, and
        // fails only once it is gone.
        let (sender, receiver) = std::sync::mpsc::sync_channel::<
            std::result::Result<StreamPayload, ai_inference::GenerationError>,
        >(STREAM_CHANNEL_CAPACITY);
        // Token counts are only known once generation ends, which is after the
        // last fragment has already gone down the channel. They therefore come
        // back beside the channel rather than through it: sending them as a
        // final fragment would make them indistinguishable from model output.
        let outcome = Arc::new(Mutex::new(ai_inference::StreamOutcome::default()));
        let generation_outcome = Arc::clone(&outcome);
        // Cleared when the guest drops its `token-stream`. A failed `send`
        // already reports a departed consumer, but only when there is something
        // to send; this is the same answer available *between* sends, which is
        // what a backend needs while it is waiting on a slow upstream.
        let consumer_alive = self
            .streaming_consumer_alive
            .as_ref()
            .map(Arc::clone)
            .unwrap_or_else(|| Arc::new(std::sync::atomic::AtomicBool::new(true)));
        let generation_alive = Arc::clone(&consumer_alive);
        let budget = Arc::new(StreamQueueBudget::default());
        let generation_budget = Arc::clone(&budget);
        std::thread::Builder::new()
            .name("tachyon-stream-gen".to_owned())
            .spawn(move || {
                // A failed `send` means the receiver is gone — the SSE client
                // disconnected and the guest dropped the stream. That is
                // reported back to the backend as `Stop` rather than ignored:
                // an ignored close leaves the upstream reader draining a
                // response nobody wants until `[DONE]` or the binding timeout,
                // holding an admission permit the whole time, so a handful of
                // abandoned streams would starve live requests.
                let mut sink = GuestStreamSink {
                    sender: &sender,
                    consumer_alive: &generation_alive,
                    budget: &generation_budget,
                };
                match ai_runtime.stream_component_prompt(
                    &alias,
                    &prompt,
                    adapter_id.as_deref(),
                    &mut sink,
                ) {
                    // An absent count means the backend could not measure, and
                    // stays absent in the slot: `usage()` then reports nothing
                    // rather than zeros, which a client would read as a
                    // generation that cost nothing. Same for the finish reason.
                    Ok(reported) => {
                        if let Ok(mut slot) = generation_outcome.lock() {
                            *slot = reported;
                        }
                    }
                    Err(error) => {
                        let _ = sender.send(Err(error));
                    }
                }
                // `sender` drops here: the channel closes and `next` reports the
                // end of the stream.
            })
            .map_err(|error| {
                ai_inference::GenerationError::local(format!(
                    "failed to spawn streaming generation thread: {error}"
                ))
            })?;
        Ok(StreamedGeneration {
            receiver,
            outcome,
            consumer_alive,
            budget,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ControlPlaneSnapshot {
    pub(crate) cpu_pressure: u8,
    pub(crate) ram_pressure: u8,
    pub(crate) active_tasks: u32,
    pub(crate) active_instances: u32,
    pub(crate) allocated_memory_pages: u32,
    pub(crate) capability_mask: u64,
    pub(crate) capabilities: Vec<String>,
    pub(crate) cpu_rt_load: u32,
    pub(crate) cpu_standard_load: u32,
    pub(crate) cpu_batch_load: u32,
    pub(crate) gpu_rt_load: u32,
    pub(crate) gpu_standard_load: u32,
    pub(crate) gpu_batch_load: u32,
    pub(crate) npu_rt_load: u32,
    pub(crate) npu_standard_load: u32,
    pub(crate) npu_batch_load: u32,
    pub(crate) tpu_rt_load: u32,
    pub(crate) tpu_standard_load: u32,
    pub(crate) tpu_batch_load: u32,
    pub(crate) network_rt_load: u32,
    pub(crate) network_standard_load: u32,
    pub(crate) network_batch_load: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct AcceleratorQueueLoads {
    pub(crate) cpu_rt_load: u32,
    pub(crate) cpu_standard_load: u32,
    pub(crate) cpu_batch_load: u32,
    pub(crate) gpu_rt_load: u32,
    pub(crate) gpu_standard_load: u32,
    pub(crate) gpu_batch_load: u32,
    pub(crate) npu_rt_load: u32,
    pub(crate) npu_standard_load: u32,
    pub(crate) npu_batch_load: u32,
    pub(crate) tpu_rt_load: u32,
    pub(crate) tpu_standard_load: u32,
    pub(crate) tpu_batch_load: u32,
    pub(crate) network_rt_load: u32,
    pub(crate) network_standard_load: u32,
    pub(crate) network_batch_load: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct RouteOverrideDescriptor {
    #[serde(default)]
    pub(crate) candidates: Vec<RouteOverrideCandidate>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct RouteOverrideCandidate {
    pub(crate) destination: String,
    #[serde(default)]
    pub(crate) hot_models: Vec<String>,
    #[serde(default)]
    pub(crate) effective_pressure: u8,
    #[serde(default)]
    pub(crate) capability_mask: u64,
    #[serde(default)]
    pub(crate) capabilities: Vec<String>,
}

pub(crate) fn guest_memory_page_count(bytes: usize) -> usize {
    ((bytes.saturating_add(65_535)) / 65_536).max(1)
}

pub(crate) fn saturating_percent(value: usize, capacity: usize) -> u8 {
    if capacity == 0 {
        return 0;
    }

    let percent = value.saturating_mul(100) / capacity;
    percent.min(100) as u8
}

pub(crate) fn control_plane_snapshot(
    telemetry: &TelemetryHandle,
    host_load: &HostLoadCounters,
    concurrency_limits: &HashMap<String, Arc<RouteExecutionControl>>,
    runtime_config: &IntegrityConfig,
    host_capabilities: Capabilities,
    queue_loads: AcceleratorQueueLoads,
) -> ControlPlaneSnapshot {
    let active_tasks = telemetry::active_requests(telemetry).min(u32::MAX as usize) as u32;
    let active_instances = host_load.active_instances.load(Ordering::SeqCst);
    let allocated_memory_pages = host_load.allocated_memory_pages.load(Ordering::SeqCst);
    let total_capacity = concurrency_limits
        .values()
        .map(|control| control.max_concurrency as usize)
        .sum::<usize>()
        .max(1);
    let total_memory_pages = total_capacity
        .saturating_mul(guest_memory_page_count(
            runtime_config.guest_memory_limit_bytes,
        ))
        .max(1);

    ControlPlaneSnapshot {
        cpu_pressure: saturating_percent(
            active_instances.max(active_tasks as usize),
            total_capacity,
        ),
        ram_pressure: saturating_percent(allocated_memory_pages, total_memory_pages),
        active_tasks,
        active_instances: active_instances.min(u32::MAX as usize) as u32,
        allocated_memory_pages: allocated_memory_pages.min(u32::MAX as usize) as u32,
        capability_mask: host_capabilities.mask,
        capabilities: host_capabilities.names(),
        cpu_rt_load: queue_loads.cpu_rt_load,
        cpu_standard_load: queue_loads.cpu_standard_load,
        cpu_batch_load: queue_loads.cpu_batch_load,
        gpu_rt_load: queue_loads.gpu_rt_load,
        gpu_standard_load: queue_loads.gpu_standard_load,
        gpu_batch_load: queue_loads.gpu_batch_load,
        npu_rt_load: queue_loads.npu_rt_load,
        npu_standard_load: queue_loads.npu_standard_load,
        npu_batch_load: queue_loads.npu_batch_load,
        tpu_rt_load: queue_loads.tpu_rt_load,
        tpu_standard_load: queue_loads.tpu_standard_load,
        tpu_batch_load: queue_loads.tpu_batch_load,
        network_rt_load: queue_loads.network_rt_load,
        network_standard_load: queue_loads.network_standard_load,
        network_batch_load: queue_loads.network_batch_load,
    }
}

pub(crate) fn control_plane_override_destination(
    route_overrides: &ArcSwap<HashMap<String, String>>,
    peer_capabilities: &PeerCapabilityCache,
    route_path: &str,
    headers: &HeaderMap,
    required_capability_mask: u64,
    requested_model: Option<&str>,
) -> Option<String> {
    if headers.contains_key(TACHYON_BUFFER_REPLAY_HEADER) {
        return None;
    }

    let raw = route_overrides
        .load()
        .get(&normalize_route_override_key(route_path))
        .cloned()?;

    if let Ok(descriptor) = serde_json::from_str::<RouteOverrideDescriptor>(&raw) {
        let selected = descriptor.candidates.iter().find(|candidate| {
            let supports_capabilities = candidate_supports_capabilities(
                candidate,
                peer_capabilities,
                required_capability_mask,
            );
            let supports_model = requested_model.is_none_or(|alias| {
                candidate
                    .hot_models
                    .iter()
                    .any(|model| model.eq_ignore_ascii_case(alias))
            });
            supports_capabilities && supports_model
        });
        return selected.map(|candidate| candidate.destination.clone());
    }

    if destination_supports_capabilities(&raw, peer_capabilities, required_capability_mask) {
        return Some(raw);
    }

    cached_capable_peer_destination(peer_capabilities, route_path, required_capability_mask)
}

pub(crate) fn update_control_plane_route_override(
    route_overrides: &ArcSwap<HashMap<String, String>>,
    peer_capabilities: &PeerCapabilityCache,
    route_path: &str,
    destination: &str,
) -> std::result::Result<(), String> {
    let normalized_route = normalize_route_override_key(route_path);
    let normalized_destination = destination.trim();
    if normalized_destination.is_empty() {
        return Err("route override destinations must not be empty".to_owned());
    }

    let direct_destination = normalized_destination.starts_with('/')
        || normalized_destination.starts_with("http://")
        || normalized_destination.starts_with("https://");
    if !direct_destination {
        let descriptor = serde_json::from_str::<RouteOverrideDescriptor>(normalized_destination)
            .map_err(|_| {
                format!(
                    "route override `{normalized_destination}` must be an absolute route, URL, or serialized candidate descriptor"
                )
            })?;
        if descriptor.candidates.is_empty() {
            return Err(
                "route override descriptors must include at least one candidate".to_owned(),
            );
        }
        for candidate in &descriptor.candidates {
            let candidate_destination = candidate.destination.trim();
            if !candidate_destination.starts_with('/')
                && !candidate_destination.starts_with("http://")
                && !candidate_destination.starts_with("https://")
            {
                return Err(format!(
                    "route override candidate `{candidate_destination}` must be an absolute route or URL"
                ));
            }
        }
        cache_peer_capabilities(peer_capabilities, &descriptor.candidates);
    }

    let mut next = (**route_overrides.load()).clone();
    if normalized_destination == normalized_route {
        next.remove(&normalized_route);
    } else {
        next.insert(normalized_route, normalized_destination.to_owned());
    }
    route_overrides.store(Arc::new(next));
    Ok(())
}

pub(crate) fn candidate_supports_capabilities(
    candidate: &RouteOverrideCandidate,
    peer_capabilities: &PeerCapabilityCache,
    required_capability_mask: u64,
) -> bool {
    if required_capability_mask == 0 || required_capability_mask == Capabilities::CORE_WASI {
        return true;
    }

    let declared_mask = required_capability_mask_for_candidate(candidate);
    if declared_mask != 0 {
        return (declared_mask & required_capability_mask) == required_capability_mask;
    }

    destination_supports_capabilities(
        &candidate.destination,
        peer_capabilities,
        required_capability_mask,
    )
}

pub(crate) fn destination_supports_capabilities(
    destination: &str,
    peer_capabilities: &PeerCapabilityCache,
    required_capability_mask: u64,
) -> bool {
    if required_capability_mask == 0 || required_capability_mask == Capabilities::CORE_WASI {
        return true;
    }

    peer_base_url_for_destination(destination)
        .and_then(|base_url| {
            peer_capabilities
                .lock()
                .expect("peer capability cache should not be poisoned")
                .get(&base_url)
                .cloned()
        })
        .is_some_and(|peer| {
            (peer.capability_mask & required_capability_mask) == required_capability_mask
        })
}

pub(crate) fn cache_peer_capabilities(
    peer_capabilities: &PeerCapabilityCache,
    candidates: &[RouteOverrideCandidate],
) {
    let mut cache = peer_capabilities
        .lock()
        .expect("peer capability cache should not be poisoned");
    for candidate in candidates {
        let Some(base_url) = peer_base_url_for_destination(&candidate.destination) else {
            continue;
        };
        let capability_mask = required_capability_mask_for_candidate(candidate);
        if capability_mask == 0 {
            continue;
        }
        let capabilities = if candidate.capabilities.is_empty() {
            capability_names_from_mask(capability_mask)
        } else {
            candidate.capabilities.clone()
        };
        cache.insert(
            base_url,
            CachedPeerCapabilities {
                capabilities,
                capability_mask,
                effective_pressure: candidate.effective_pressure,
            },
        );
    }
}

pub(crate) fn required_capability_mask_for_candidate(candidate: &RouteOverrideCandidate) -> u64 {
    if candidate.capability_mask != 0 {
        return candidate.capability_mask;
    }
    Capabilities::from_requirement_list(&candidate.capabilities)
        .map(|capabilities| capabilities.mask)
        .unwrap_or(0)
}

pub(crate) fn peer_base_url_for_destination(destination: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(destination).ok()?;
    let host = parsed.host_str()?;
    let mut base = format!("{}://{}", parsed.scheme(), host);
    if let Some(port) = parsed.port() {
        base.push(':');
        base.push_str(&port.to_string());
    }
    Some(base)
}

pub(crate) fn cached_capable_peer_destination(
    peer_capabilities: &PeerCapabilityCache,
    route_path: &str,
    required_capability_mask: u64,
) -> Option<String> {
    let cache = peer_capabilities
        .lock()
        .expect("peer capability cache should not be poisoned");
    cache
        .iter()
        .filter(|(_, peer)| {
            (peer.capability_mask & required_capability_mask) == required_capability_mask
        })
        .min_by_key(|(_, peer)| peer.effective_pressure)
        .map(|(base_url, _)| format!("{base_url}{}", route_path_for_override_key(route_path)))
}

pub(crate) fn build_component_wasi_ctx(
    route: &IntegrityRoute,
    host_identity: &HostIdentity,
    s3_preps: &[crate::host_core::volumes::S3VolumePrep],
) -> std::result::Result<WasiCtx, ExecutionError> {
    // Intentionally do not inherit the host environment. Secrets stay in host memory
    // and are only reachable through the typed vault import.
    let mut wasi = WasiCtxBuilder::new();
    for (name, value) in system_runtime_environment(route, host_identity) {
        wasi.env(&name, &value);
    }
    preopen_route_volumes(&mut wasi, route)?;
    preopen_s3_volume_dirs(&mut wasi, s3_preps)?;
    preopen_gitops_config_store(&mut wasi, route)?;
    Ok(wasi.build())
}

pub(crate) fn add_route_environment(
    wasi: &mut WasiCtxBuilder,
    route: &IntegrityRoute,
    host_identity: &HostIdentity,
) -> std::result::Result<(), ExecutionError> {
    add_route_environment_with_trace(wasi, route, host_identity, None)
}

pub(crate) fn add_route_environment_with_trace(
    wasi: &mut WasiCtxBuilder,
    route: &IntegrityRoute,
    host_identity: &HostIdentity,
    traceparent: Option<&str>,
) -> std::result::Result<(), ExecutionError> {
    for (name, value) in system_runtime_environment(route, host_identity) {
        wasi.env(&name, &value);
    }
    if let Some(tp) = traceparent {
        // W3C Trace Context propagation. Guests opt in by reading `TRACEPARENT` from
        // their environment (the `faas-sdk` logger / metrics macros do so transparently).
        wasi.env(TACHYON_TRACEPARENT_ENV, tp);
    }
    Ok(())
}

pub(crate) fn system_runtime_environment(
    route: &IntegrityRoute,
    host_identity: &HostIdentity,
) -> Vec<(String, String)> {
    let mut env = route
        .env
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Vec<_>>();
    if route.role == RouteRole::System {
        env.push((
            TACHYON_SYSTEM_PUBLIC_KEY_ENV.to_owned(),
            host_identity.public_key_hex.clone(),
        ));
    }
    env
}

/// Standard W3C Trace Context environment variable name. Guests read it via WASI to
/// obtain the active trace context for the request they are servicing.
pub(crate) const TACHYON_TRACEPARENT_ENV: &str = "TRACEPARENT";

/// Honor the inbound `traceparent` header if it is well-formed per the W3C Trace
/// Context spec, otherwise mint a fresh one via the existing `generate_traceparent`
/// so every request that reaches the host gets a globally identifiable trace id.
pub(crate) fn trace_context_for_request(headers: &HeaderMap) -> String {
    if let Some(value) = headers.get("traceparent") {
        if let Ok(s) = value.to_str() {
            if is_valid_w3c_traceparent(s) {
                return s.to_owned();
            }
        }
    }
    generate_traceparent()
}

pub(crate) fn is_valid_w3c_traceparent(value: &str) -> bool {
    // Format: VERSION-TRACE_ID-PARENT_ID-FLAGS, hex; widths 2-32-16-2.
    let parts: Vec<&str> = value.split('-').collect();
    if parts.len() != 4 {
        return false;
    }
    let widths = [2usize, 32, 16, 2];
    for (part, width) in parts.iter().zip(widths.iter()) {
        if part.len() != *width || !part.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
    }
    // Forbid the reserved all-zero trace and span ids.
    if parts[1].chars().all(|c| c == '0') || parts[2].chars().all(|c| c == '0') {
        return false;
    }
    true
}

pub(crate) fn preopen_s3_volume_dirs(
    wasi: &mut WasiCtxBuilder,
    preps: &[crate::host_core::volumes::S3VolumePrep],
) -> std::result::Result<(), ExecutionError> {
    for prep in preps {
        wasi.preopened_dir(
            &prep.temp_path,
            &prep.guest_path,
            volume_dir_perms(prep.readonly),
            volume_file_perms(prep.readonly),
        )
        .map_err(|error| {
            ExecutionError::Internal(format!(
                "failed to preopen S3 volume temp dir `{}` at guest path `{}`: {error}",
                prep.temp_path.display(),
                prep.guest_path,
            ))
        })?;
    }
    Ok(())
}

pub(crate) fn preopen_route_volumes(
    wasi: &mut WasiCtxBuilder,
    route: &IntegrityRoute,
) -> std::result::Result<(), ExecutionError> {
    for volume in &route.volumes {
        // S3 volumes are preopened separately via preopen_s3_volume_dirs after
        // async download into a per-invocation temp dir.
        if volume.volume_type.is_s3() {
            continue;
        }
        if volume.volume_type == VolumeType::Ram {
            fs::create_dir_all(&volume.host_path).map_err(|error| {
                ExecutionError::Internal(format!(
                    "failed to initialize RAM volume `{}` for route `{}`: {error}",
                    volume.host_path, route.path
                ))
            })?;
        }
        let host_path = if volume.encrypted {
            encrypted_volume_host_path(&volume.host_path)
        } else {
            PathBuf::from(&volume.host_path)
        };
        if volume.encrypted {
            fs::create_dir_all(&host_path).map_err(|error| {
                ExecutionError::Internal(format!(
                    "failed to initialize encrypted volume `{}` for route `{}`: {error}",
                    host_path.display(),
                    route.path
                ))
            })?;
        }
        wasi.preopened_dir(
            &host_path,
            &volume.guest_path,
            volume_dir_perms(volume.readonly),
            volume_file_perms(volume.readonly),
        )
        .map_err(|error| {
            ExecutionError::Internal(format!(
                "failed to preopen volume `{}` for route `{}` at guest path `{}`: {error}",
                host_path.display(),
                route.path,
                volume.guest_path
            ))
        })?;
    }

    Ok(())
}

pub(crate) fn preopen_gitops_config_store(
    wasi: &mut WasiCtxBuilder,
    route: &IntegrityRoute,
) -> std::result::Result<(), ExecutionError> {
    if !is_gitops_broker_route(route)
        || route_preopens_guest_path(route, GITOPS_CONFIG_STORE_GUEST_PATH)
    {
        return Ok(());
    }

    let host_path = gitops_config_store_host_path();
    fs::create_dir_all(&host_path).map_err(|error| {
        ExecutionError::Internal(format!(
            "failed to initialize GitOps config store `{}` for route `{}`: {error}",
            host_path.display(),
            route.path
        ))
    })?;
    wasi.preopened_dir(
        &host_path,
        GITOPS_CONFIG_STORE_GUEST_PATH,
        volume_dir_perms(false),
        volume_file_perms(false),
    )
    .map_err(|error| {
        ExecutionError::Internal(format!(
            "failed to preopen GitOps config store `{}` for route `{}` at guest path `{}`: {error}",
            host_path.display(),
            route.path,
            GITOPS_CONFIG_STORE_GUEST_PATH
        ))
    })?;

    Ok(())
}

pub(crate) fn is_gitops_broker_route(route: &IntegrityRoute) -> bool {
    route.role == RouteRole::System
        && (route.path == SYSTEM_GITOPS_BROKER_ROUTE
            || route.name == SYSTEM_GITOPS_BROKER_MODULE
            || route
                .targets
                .iter()
                .any(|target| target.module == SYSTEM_GITOPS_BROKER_MODULE))
}

pub(crate) fn route_preopens_guest_path(route: &IntegrityRoute, guest_path: &str) -> bool {
    let normalized = guest_path.trim_end_matches('/');
    route
        .volumes
        .iter()
        .any(|volume| volume.guest_path.trim_end_matches('/') == normalized)
}

pub(crate) fn gitops_config_store_host_path() -> PathBuf {
    std::env::var(TACHYON_CONFIG_STORE_DIR_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_GITOPS_CONFIG_STORE_PATH))
}

pub(crate) fn preopen_batch_target_volumes(
    wasi: &mut WasiCtxBuilder,
    target: &IntegrityBatchTarget,
) -> Result<()> {
    for volume in &target.volumes {
        if volume.volume_type == VolumeType::Ram {
            fs::create_dir_all(&volume.host_path).with_context(|| {
                format!(
                    "failed to initialize RAM volume `{}` for batch target `{}`",
                    volume.host_path, target.name
                )
            })?;
        }
        let host_path = if volume.encrypted {
            encrypted_volume_host_path(&volume.host_path)
        } else {
            PathBuf::from(&volume.host_path)
        };
        if volume.encrypted {
            fs::create_dir_all(&host_path).with_context(|| {
                format!(
                    "failed to initialize encrypted volume `{}` for batch target `{}`",
                    host_path.display(),
                    target.name
                )
            })?;
        }

        wasi.preopened_dir(
            &host_path,
            &volume.guest_path,
            volume_dir_perms(volume.readonly),
            volume_file_perms(volume.readonly),
        )
        .map_err(|error| {
            anyhow!(
                "failed to preopen volume `{}` for batch target `{}` at guest path `{}`: {error}",
                host_path.display(),
                target.name,
                volume.guest_path
            )
        })?;
    }

    Ok(())
}

pub(crate) fn encrypted_volume_host_path(host_path: &str) -> PathBuf {
    PathBuf::from(host_path).join(".tachyon-tde")
}

pub(crate) fn prepare_encrypted_route_volumes(
    route: &IntegrityRoute,
) -> std::result::Result<(), ExecutionError> {
    for volume in route.volumes.iter().filter(|volume| volume.encrypted) {
        transform_encrypted_volume_files(&encrypted_volume_host_path(&volume.host_path), false)
            .map_err(|error| {
                ExecutionError::Internal(format!(
                    "failed to decrypt encrypted volume `{}` for route `{}`: {error:#}",
                    volume.host_path, route.path
                ))
            })?;
    }
    Ok(())
}

pub(crate) fn seal_encrypted_route_volumes(
    route: &IntegrityRoute,
) -> std::result::Result<(), ExecutionError> {
    for volume in route.volumes.iter().filter(|volume| volume.encrypted) {
        transform_encrypted_volume_files(&encrypted_volume_host_path(&volume.host_path), true)
            .map_err(|error| {
                ExecutionError::Internal(format!(
                    "failed to encrypt encrypted volume `{}` for route `{}`: {error:#}",
                    volume.host_path, route.path
                ))
            })?;
    }
    Ok(())
}

pub(crate) fn transform_encrypted_volume_files(root: &Path, encrypt: bool) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(root)
        .with_context(|| format!("failed to read encrypted volume `{}`", root.display()))?
    {
        let entry = entry.with_context(|| {
            format!(
                "failed to inspect encrypted volume entry under `{}`",
                root.display()
            )
        })?;
        let path = entry.path();
        if path.is_dir() {
            transform_encrypted_volume_files(&path, encrypt)?;
        } else if path.is_file() {
            transform_tde_file(&path, encrypt)?;
        }
    }

    Ok(())
}

pub(crate) fn transform_tde_file(path: &Path, encrypt: bool) -> Result<()> {
    let body =
        fs::read(path).with_context(|| format!("failed to read TDE file `{}`", path.display()))?;
    let transformed = if encrypt {
        encrypt_tde_file_body(&body)
    } else {
        decrypt_tde_file_body(&body)
    }?;
    if transformed != body {
        fs::write(path, transformed)
            .with_context(|| format!("failed to write TDE file `{}`", path.display()))?;
    }
    Ok(())
}

pub(crate) fn encrypt_tde_file_body(plaintext: &[u8]) -> Result<Vec<u8>> {
    if plaintext.starts_with(TDE_FILE_MAGIC) {
        return Ok(plaintext.to_vec());
    }

    let mut nonce = [0_u8; 12];
    nonce[4..].copy_from_slice(&rand::rng().random::<u64>().to_be_bytes());
    let nonce = Nonce::try_from(nonce.as_slice()).map_err(|_| anyhow!("invalid TDE nonce"))?;
    let ciphertext = tde_cipher()?
        .encrypt(&nonce, plaintext)
        .map_err(|_| anyhow!("failed to encrypt TDE file body"))?;
    let mut out = Vec::with_capacity(TDE_FILE_MAGIC.len() + nonce.len() + ciphertext.len());
    out.extend_from_slice(TDE_FILE_MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

pub(crate) fn decrypt_tde_file_body(body: &[u8]) -> Result<Vec<u8>> {
    let Some(rest) = body.strip_prefix(TDE_FILE_MAGIC) else {
        return Ok(body.to_vec());
    };
    if rest.len() < 12 {
        return Err(anyhow!("TDE file body is missing nonce"));
    }
    let (nonce, ciphertext) = rest.split_at(12);
    let nonce = Nonce::try_from(nonce).map_err(|_| anyhow!("invalid TDE nonce"))?;
    tde_cipher()?
        .decrypt(&nonce, ciphertext)
        .map_err(|_| anyhow!("failed to decrypt TDE file body"))
}

pub(crate) fn tde_cipher() -> Result<Aes256Gcm> {
    Ok(Aes256Gcm::new((&tde_key_bytes()?).into()))
}

/// Resolve the host-side TDE volume-encryption key strictly from `TDE_KEY_HEX`.
///
/// Fail-closed: refuse to seal or open an encrypted volume when no key is
/// configured instead of falling back to a constant. A default key would
/// "encrypt" sensitive volumes under a value compiled into every binary, which
/// defeats the encryption entirely. Only routes that opt into encrypted volumes
/// reach this path, so unencrypted deployments are unaffected.
pub(crate) fn tde_key_bytes() -> Result<[u8; 32]> {
    let value = std::env::var(TDE_KEY_HEX_ENV).map_err(|_| {
        anyhow!(
            "{TDE_KEY_HEX_ENV} is not configured; refusing to use a default encrypted-volume key"
        )
    })?;
    decode_tde_key_hex(value.trim())
}

pub(crate) fn decode_tde_key_hex(value: &str) -> Result<[u8; 32]> {
    if value.len() != 64 {
        return Err(anyhow!("TDE key must be 64 hexadecimal characters"));
    }
    let mut out = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks(2).enumerate() {
        let pair = std::str::from_utf8(chunk).context("TDE key must be UTF-8 hex")?;
        out[index] = u8::from_str_radix(pair, 16).context("TDE key must be hexadecimal")?;
    }
    Ok(out)
}

pub(crate) fn volume_dir_perms(readonly: bool) -> DirPerms {
    if readonly {
        DirPerms::READ
    } else {
        DirPerms::READ | DirPerms::MUTATE
    }
}

pub(crate) fn volume_file_perms(readonly: bool) -> FilePerms {
    if readonly {
        FilePerms::READ
    } else {
        FilePerms::READ | FilePerms::WRITE
    }
}

impl WasiView for ComponentHostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl WasiView for BatchCommandState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl wasmtime::component::HasData for ComponentHostState {
    type Data<'a> = &'a mut Self;
}

impl component_bindings::tachyon::mesh::secrets_vault::Host for ComponentHostState {
    fn get_secret(
        &mut self,
        name: String,
    ) -> std::result::Result<String, component_bindings::tachyon::mesh::secrets_vault::Error> {
        if !self.scopes.check_secrets(&name) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Secrets,
                &self.route_path,
            );
            return Err(component_bindings::tachyon::mesh::secrets_vault::Error::PermissionDenied);
        }
        self.secrets.get_secret(&name).map_err(|error| match error {
            SecretAccessErrorKind::NotFound => {
                component_bindings::tachyon::mesh::secrets_vault::Error::NotFound
            }
            SecretAccessErrorKind::PermissionDenied => {
                component_bindings::tachyon::mesh::secrets_vault::Error::PermissionDenied
            }
            #[cfg(not(feature = "secrets-vault"))]
            SecretAccessErrorKind::VaultDisabled => {
                component_bindings::tachyon::mesh::secrets_vault::Error::VaultDisabled
            }
        })
    }
}

impl udp_component_bindings::tachyon::mesh::secrets_vault::Host for ComponentHostState {
    fn get_secret(
        &mut self,
        name: String,
    ) -> std::result::Result<String, udp_component_bindings::tachyon::mesh::secrets_vault::Error>
    {
        if !self.scopes.check_secrets(&name) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Secrets,
                &self.route_path,
            );
            return Err(
                udp_component_bindings::tachyon::mesh::secrets_vault::Error::PermissionDenied,
            );
        }
        self.secrets.get_secret(&name).map_err(|error| match error {
            SecretAccessErrorKind::NotFound => {
                udp_component_bindings::tachyon::mesh::secrets_vault::Error::NotFound
            }
            SecretAccessErrorKind::PermissionDenied => {
                udp_component_bindings::tachyon::mesh::secrets_vault::Error::PermissionDenied
            }
            #[cfg(not(feature = "secrets-vault"))]
            SecretAccessErrorKind::VaultDisabled => {
                udp_component_bindings::tachyon::mesh::secrets_vault::Error::VaultDisabled
            }
        })
    }
}

#[cfg(feature = "websockets")]
impl websocket_component_bindings::tachyon::mesh::secrets_vault::Host for ComponentHostState {
    fn get_secret(
        &mut self,
        name: String,
    ) -> std::result::Result<
        String,
        websocket_component_bindings::tachyon::mesh::secrets_vault::Error,
    > {
        if !self.scopes.check_secrets(&name) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Secrets,
                &self.route_path,
            );
            return Err(
                websocket_component_bindings::tachyon::mesh::secrets_vault::Error::PermissionDenied,
            );
        }
        self.secrets.get_secret(&name).map_err(|error| match error {
            SecretAccessErrorKind::NotFound => {
                websocket_component_bindings::tachyon::mesh::secrets_vault::Error::NotFound
            }
            SecretAccessErrorKind::PermissionDenied => {
                websocket_component_bindings::tachyon::mesh::secrets_vault::Error::PermissionDenied
            }
            #[cfg(not(feature = "secrets-vault"))]
            SecretAccessErrorKind::VaultDisabled => {
                websocket_component_bindings::tachyon::mesh::secrets_vault::Error::VaultDisabled
            }
        })
    }
}

impl component_bindings::tachyon::mesh::bridge_controller::Host for ComponentHostState {
    fn create_bridge(
        &mut self,
        config: component_bindings::tachyon::mesh::bridge_controller::BridgeConfig,
    ) -> std::result::Result<
        component_bindings::tachyon::mesh::bridge_controller::BridgeEndpoints,
        String,
    > {
        // Scope check at construction — handle-bound; no re-check on destroy_bridge.
        if !self.scopes.check_bridge(&config.client_a_addr)
            || !self.scopes.check_bridge(&config.client_b_addr)
        {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Bridge,
                &self.route_path,
            );
            return Err(
                "permission denied: bridge address not granted by deployment scopes".to_owned(),
            );
        }
        let allocation = self.handle_bridge_create(BridgeConfig {
            client_a_addr: config.client_a_addr,
            client_b_addr: config.client_b_addr,
            timeout_seconds: config.timeout_seconds,
        })?;
        Ok(
            component_bindings::tachyon::mesh::bridge_controller::BridgeEndpoints {
                bridge_id: allocation.bridge_id,
                ip: allocation.ip,
                port_a: allocation.port_a,
                port_b: allocation.port_b,
            },
        )
    }

    fn destroy_bridge(&mut self, bridge_id: String) -> std::result::Result<(), String> {
        self.handle_bridge_destroy(&bridge_id)
    }
}

impl system_component_bindings::tachyon::mesh::bridge_controller::Host for ComponentHostState {
    fn create_bridge(
        &mut self,
        config: system_component_bindings::tachyon::mesh::bridge_controller::BridgeConfig,
    ) -> std::result::Result<
        system_component_bindings::tachyon::mesh::bridge_controller::BridgeEndpoints,
        String,
    > {
        if !self.scopes.check_bridge(&config.client_a_addr)
            || !self.scopes.check_bridge(&config.client_b_addr)
        {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Bridge,
                &self.route_path,
            );
            return Err(
                "permission denied: bridge address not granted by deployment scopes".to_owned(),
            );
        }
        let allocation = self.handle_bridge_create(BridgeConfig {
            client_a_addr: config.client_a_addr,
            client_b_addr: config.client_b_addr,
            timeout_seconds: config.timeout_seconds,
        })?;
        Ok(
            system_component_bindings::tachyon::mesh::bridge_controller::BridgeEndpoints {
                bridge_id: allocation.bridge_id,
                ip: allocation.ip,
                port_a: allocation.port_a,
                port_b: allocation.port_b,
            },
        )
    }

    fn destroy_bridge(&mut self, bridge_id: String) -> std::result::Result<(), String> {
        self.handle_bridge_destroy(&bridge_id)
    }
}

impl component_bindings::tachyon::mesh::vector::Host for ComponentHostState {
    fn create_index(
        &mut self,
        spec: component_bindings::tachyon::mesh::vector::IndexSpec,
    ) -> std::result::Result<(), String> {
        if !self.scopes.check_vector(&spec.name) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Vector,
                &self.route_path,
            );
            return Err(format!(
                "permission denied: vector index `{}` not granted by deployment scopes",
                spec.name
            ));
        }
        self.storage_broker
            .core_store
            .create_vector_index(
                &self.vector_tenant_id(),
                &spec.name,
                spec.dim as usize,
                spec.m,
                spec.ef_construction,
            )
            .map_err(|error| format!("{error:#}"))
    }

    fn upsert(
        &mut self,
        name: String,
        docs: Vec<component_bindings::tachyon::mesh::vector::Document>,
    ) -> std::result::Result<(), String> {
        if !self.scopes.check_vector(&name) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Vector,
                &self.route_path,
            );
            return Err(format!(
                "permission denied: vector index `{name}` not granted by deployment scopes"
            ));
        }
        let docs = docs
            .into_iter()
            .map(|doc| store::VectorDocument {
                id: doc.id,
                embedding: doc.embedding,
                payload: doc.payload,
            })
            .collect();
        self.storage_broker
            .core_store
            .upsert_vectors(&self.vector_tenant_id(), &name, docs)
            .map_err(|error| format!("{error:#}"))
    }

    fn search(
        &mut self,
        name: String,
        query: Vec<f32>,
        k: u32,
    ) -> std::result::Result<Vec<component_bindings::tachyon::mesh::vector::SearchMatch>, String>
    {
        if !self.scopes.check_vector(&name) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Vector,
                &self.route_path,
            );
            return Err(format!(
                "permission denied: vector index `{name}` not granted by deployment scopes"
            ));
        }
        self.storage_broker
            .core_store
            .search_vectors(&self.vector_tenant_id(), &name, &query, k as usize)
            .map(|matches| {
                matches
                    .into_iter()
                    .map(
                        |item| component_bindings::tachyon::mesh::vector::SearchMatch {
                            id: item.id,
                            score: item.score,
                            payload: item.payload,
                        },
                    )
                    .collect()
            })
            .map_err(|error| format!("{error:#}"))
    }

    fn remove(&mut self, name: String, id: String) -> std::result::Result<bool, String> {
        if !self.scopes.check_vector(&name) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Vector,
                &self.route_path,
            );
            return Err(format!(
                "permission denied: vector index `{name}` not granted by deployment scopes"
            ));
        }
        self.storage_broker
            .core_store
            .remove_vector(&self.vector_tenant_id(), &name, &id)
            .map_err(|error| format!("{error:#}"))
    }
}

impl component_bindings::tachyon::mesh::training::Host for ComponentHostState {
    fn submit_training_job(
        &mut self,
        job: component_bindings::tachyon::mesh::training::TrainingJob,
    ) -> std::result::Result<component_bindings::tachyon::mesh::training::JobId, String> {
        if !self.scopes.check_training(&job.dataset.volume_alias) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Training,
                &self.route_path,
            );
            return Err(format!(
                "permission denied: training volume alias `{}` not granted by deployment scopes",
                job.dataset.volume_alias
            ));
        }
        if job.base_model.trim().is_empty() {
            return Err("training job base model must not be empty".to_owned());
        }
        if job.dataset.path.trim().is_empty() {
            return Err("training job dataset path must not be empty".to_owned());
        }
        let queue = lora_training_queue();
        let id = format!("lora-{}", Uuid::new_v4().simple());
        update_lora_training_status(&queue.statuses, &id, LoraTrainingJobStatus::Queued);
        queue
            .sender
            .send(LoraTrainingJob {
                id: id.clone(),
                tenant_id: self.vector_tenant_id(),
                base_model: job.base_model,
                dataset_volume: job.dataset.volume_alias,
                dataset_path: job.dataset.path,
                dataset_split: job.dataset.split,
                rank: job.rank,
                max_steps: job.max_steps,
                seed: job.seed,
            })
            .map_err(|error| format!("failed to queue LoRA training job: {error}"))?;
        Ok(component_bindings::tachyon::mesh::training::JobId { value: id })
    }

    fn get_training_status(
        &mut self,
        id: component_bindings::tachyon::mesh::training::JobId,
    ) -> std::result::Result<component_bindings::tachyon::mesh::training::JobStatus, String> {
        let queue = lora_training_queue();
        let status = queue
            .statuses
            .lock()
            .expect("LoRA training status map should not be poisoned")
            .get(&id.value)
            .cloned()
            .ok_or_else(|| format!("unknown LoRA training job `{}`", id.value))?;
        Ok(match status {
            LoraTrainingJobStatus::Queued => {
                component_bindings::tachyon::mesh::training::JobStatus::Queued
            }
            LoraTrainingJobStatus::Running { step, total } => {
                component_bindings::tachyon::mesh::training::JobStatus::Running((step, total))
            }
            LoraTrainingJobStatus::Completed { adapter_path } => {
                component_bindings::tachyon::mesh::training::JobStatus::Completed(adapter_path)
            }
            LoraTrainingJobStatus::Failed { message } => {
                component_bindings::tachyon::mesh::training::JobStatus::Failed(message)
            }
        })
    }
}

impl component_bindings::tachyon::mesh::kv_partition::Host for ComponentHostState {}

impl control_plane_component_bindings::tachyon::mesh::kv_partition::Host for ComponentHostState {}

// ── graph::workspace-graph Wasmtime host bindings ────────────────────────────

impl component_bindings::tachyon::mesh::graph::Host for ComponentHostState {}

impl component_bindings::tachyon::mesh::graph::HostWorkspaceGraph for ComponentHostState {
    fn new(
        &mut self,
        name: String,
    ) -> wasmtime::component::Resource<component_bindings::tachyon::mesh::graph::WorkspaceGraph>
    {
        // Scope check at construction only; subsequent methods on this handle do not re-check.
        let scope_denial = if !self.scopes.check_graph(&name) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Graph,
                &self.route_path,
            );
            Some(format!(
                "permission denied: graph workspace `{name}` not granted by deployment scopes"
            ))
        } else {
            None
        };
        let resource = WorkspaceGraphResource {
            graph_name: name,
            core_store: Arc::clone(&self.storage_broker.core_store),
            scope_denial,
        };
        let owned: wasmtime::component::Resource<WorkspaceGraphResource> = self
            .table
            .push(resource)
            .expect("resource table push failed");
        wasmtime::component::Resource::new_own(owned.rep())
    }

    fn add_edges(
        &mut self,
        self_: wasmtime::component::Resource<
            component_bindings::tachyon::mesh::graph::WorkspaceGraph,
        >,
        edges: Vec<component_bindings::tachyon::mesh::graph::Edge>,
    ) -> std::result::Result<(), String> {
        let handle =
            wasmtime::component::Resource::<WorkspaceGraphResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        let graph_edges: Vec<GraphEdge> = edges
            .into_iter()
            .map(|e| GraphEdge {
                subject: e.subject,
                predicate: e.predicate,
                object: e.object,
                properties: e.properties,
            })
            .collect();
        res.core_store
            .graph_add_edges(&res.graph_name, &graph_edges)
            .map_err(|e| format!("{e:#}"))
    }

    fn delete_edges(
        &mut self,
        self_: wasmtime::component::Resource<
            component_bindings::tachyon::mesh::graph::WorkspaceGraph,
        >,
        edges: Vec<component_bindings::tachyon::mesh::graph::Edge>,
    ) -> std::result::Result<(), String> {
        let handle =
            wasmtime::component::Resource::<WorkspaceGraphResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        let graph_edges: Vec<GraphEdge> = edges
            .into_iter()
            .map(|e| GraphEdge {
                subject: e.subject,
                predicate: e.predicate,
                object: e.object,
                properties: e.properties,
            })
            .collect();
        res.core_store
            .graph_delete_edges(&res.graph_name, &graph_edges)
            .map_err(|e| format!("{e:#}"))
    }

    fn traverse(
        &mut self,
        self_: wasmtime::component::Resource<
            component_bindings::tachyon::mesh::graph::WorkspaceGraph,
        >,
        subject: String,
        predicate: String,
        depth: u32,
    ) -> std::result::Result<Vec<String>, String> {
        let handle =
            wasmtime::component::Resource::<WorkspaceGraphResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        res.core_store
            .graph_traverse(&res.graph_name, &subject, &predicate, depth)
            .map_err(|e| format!("{e:#}"))
    }

    fn drop(
        &mut self,
        rep: wasmtime::component::Resource<
            component_bindings::tachyon::mesh::graph::WorkspaceGraph,
        >,
    ) -> wasmtime::Result<()> {
        self.table
            .delete(wasmtime::component::Resource::<WorkspaceGraphResource>::new_own(rep.rep()))?;
        Ok(())
    }
}

impl component_bindings::tachyon::mesh::kv_partition::HostTable for ComponentHostState {
    fn new(
        &mut self,
        name: String,
    ) -> wasmtime::component::Resource<component_bindings::tachyon::mesh::kv_partition::Table> {
        // Scope check at construction only; subsequent get/set/delete calls on this handle
        // do not re-check (handle-bound invariant).
        let scope_denial = if !self.scopes.check_kv(&name) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Kv,
                &self.route_path,
            );
            Some(format!(
                "permission denied: kv table `{name}` not granted by deployment scopes"
            ))
        } else {
            None
        };
        let resource = RedbTableResource {
            table_name: name,
            core_store: Arc::clone(&self.storage_broker.core_store),
            scope_denial,
        };
        let owned: wasmtime::component::Resource<RedbTableResource> = self
            .table
            .push(resource)
            .expect("resource table push failed");
        wasmtime::component::Resource::new_own(owned.rep())
    }

    fn get(
        &mut self,
        self_: wasmtime::component::Resource<
            component_bindings::tachyon::mesh::kv_partition::Table,
        >,
        key: String,
    ) -> std::result::Result<Vec<u8>, String> {
        let handle = wasmtime::component::Resource::<RedbTableResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        res.core_store
            .kv_partition_get(&res.table_name, &key)
            .map_err(|e| format!("{e:#}"))
            .and_then(|opt| opt.ok_or_else(|| format!("key `{key}` not found")))
    }

    fn set(
        &mut self,
        self_: wasmtime::component::Resource<
            component_bindings::tachyon::mesh::kv_partition::Table,
        >,
        key: String,
        value: Vec<u8>,
    ) -> std::result::Result<(), String> {
        let handle = wasmtime::component::Resource::<RedbTableResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        res.core_store
            .kv_partition_set(&res.table_name, &key, &value)
            .map_err(|e| format!("{e:#}"))
    }

    fn compare_and_set(
        &mut self,
        self_: wasmtime::component::Resource<
            component_bindings::tachyon::mesh::kv_partition::Table,
        >,
        key: String,
        expected: Option<Vec<u8>>,
        value: Vec<u8>,
    ) -> std::result::Result<bool, String> {
        let handle = wasmtime::component::Resource::<RedbTableResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        res.core_store
            .kv_partition_compare_and_set(&res.table_name, &key, expected.as_deref(), &value)
            .map_err(|e| format!("{e:#}"))
    }

    fn compare_and_delete(
        &mut self,
        self_: wasmtime::component::Resource<
            component_bindings::tachyon::mesh::kv_partition::Table,
        >,
        key: String,
        expected: Vec<u8>,
    ) -> std::result::Result<bool, String> {
        let handle = wasmtime::component::Resource::<RedbTableResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        res.core_store
            .kv_partition_compare_and_delete(&res.table_name, &key, &expected)
            .map_err(|e| format!("{e:#}"))
    }

    fn delete(
        &mut self,
        self_: wasmtime::component::Resource<
            component_bindings::tachyon::mesh::kv_partition::Table,
        >,
        key: String,
    ) -> std::result::Result<(), String> {
        let handle = wasmtime::component::Resource::<RedbTableResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        res.core_store
            .kv_partition_delete(&res.table_name, &key)
            .map_err(|e| format!("{e:#}"))
    }

    fn batch_set(
        &mut self,
        self_: wasmtime::component::Resource<
            component_bindings::tachyon::mesh::kv_partition::Table,
        >,
        entries: Vec<(String, Vec<u8>)>,
    ) -> std::result::Result<(), String> {
        let handle = wasmtime::component::Resource::<RedbTableResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        res.core_store
            .kv_partition_batch_set(&res.table_name, &entries)
            .map_err(|e| format!("{e:#}"))
    }

    fn get_range(
        &mut self,
        self_: wasmtime::component::Resource<
            component_bindings::tachyon::mesh::kv_partition::Table,
        >,
        start_key: String,
        end_key: String,
        limit: u32,
        offset: u32,
    ) -> std::result::Result<Vec<(String, Vec<u8>)>, String> {
        let handle = wasmtime::component::Resource::<RedbTableResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        res.core_store
            .kv_partition_get_range(&res.table_name, &start_key, &end_key, limit, offset)
            .map_err(|e| format!("{e:#}"))
    }

    fn drop(
        &mut self,
        rep: wasmtime::component::Resource<component_bindings::tachyon::mesh::kv_partition::Table>,
    ) -> wasmtime::Result<()> {
        self.table
            .delete(wasmtime::component::Resource::<RedbTableResource>::new_own(
                rep.rep(),
            ))?;
        Ok(())
    }
}

impl control_plane_component_bindings::tachyon::mesh::kv_partition::HostTable
    for ComponentHostState
{
    fn new(
        &mut self,
        name: String,
    ) -> wasmtime::component::Resource<
        control_plane_component_bindings::tachyon::mesh::kv_partition::Table,
    > {
        // Scope check at construction only; subsequent get/set/delete calls on this handle
        // do not re-check (handle-bound invariant).
        let scope_denial = if !self.scopes.check_kv(&name) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Kv,
                &self.route_path,
            );
            Some(format!(
                "permission denied: kv table `{name}` not granted by deployment scopes"
            ))
        } else {
            None
        };
        let resource = RedbTableResource {
            table_name: name,
            core_store: Arc::clone(&self.storage_broker.core_store),
            scope_denial,
        };
        let owned: wasmtime::component::Resource<RedbTableResource> = self
            .table
            .push(resource)
            .expect("resource table push failed");
        wasmtime::component::Resource::new_own(owned.rep())
    }

    fn get(
        &mut self,
        self_: wasmtime::component::Resource<
            control_plane_component_bindings::tachyon::mesh::kv_partition::Table,
        >,
        key: String,
    ) -> std::result::Result<Vec<u8>, String> {
        let handle = wasmtime::component::Resource::<RedbTableResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        res.core_store
            .kv_partition_get(&res.table_name, &key)
            .map_err(|e| format!("{e:#}"))
            .and_then(|opt| opt.ok_or_else(|| format!("key `{key}` not found")))
    }

    fn set(
        &mut self,
        self_: wasmtime::component::Resource<
            control_plane_component_bindings::tachyon::mesh::kv_partition::Table,
        >,
        key: String,
        value: Vec<u8>,
    ) -> std::result::Result<(), String> {
        let handle = wasmtime::component::Resource::<RedbTableResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        res.core_store
            .kv_partition_set(&res.table_name, &key, &value)
            .map_err(|e| format!("{e:#}"))
    }

    fn compare_and_set(
        &mut self,
        self_: wasmtime::component::Resource<
            control_plane_component_bindings::tachyon::mesh::kv_partition::Table,
        >,
        key: String,
        expected: Option<Vec<u8>>,
        value: Vec<u8>,
    ) -> std::result::Result<bool, String> {
        let handle = wasmtime::component::Resource::<RedbTableResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        res.core_store
            .kv_partition_compare_and_set(&res.table_name, &key, expected.as_deref(), &value)
            .map_err(|e| format!("{e:#}"))
    }

    fn compare_and_delete(
        &mut self,
        self_: wasmtime::component::Resource<
            control_plane_component_bindings::tachyon::mesh::kv_partition::Table,
        >,
        key: String,
        expected: Vec<u8>,
    ) -> std::result::Result<bool, String> {
        let handle = wasmtime::component::Resource::<RedbTableResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        res.core_store
            .kv_partition_compare_and_delete(&res.table_name, &key, &expected)
            .map_err(|e| format!("{e:#}"))
    }

    fn delete(
        &mut self,
        self_: wasmtime::component::Resource<
            control_plane_component_bindings::tachyon::mesh::kv_partition::Table,
        >,
        key: String,
    ) -> std::result::Result<(), String> {
        let handle = wasmtime::component::Resource::<RedbTableResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        res.core_store
            .kv_partition_delete(&res.table_name, &key)
            .map_err(|e| format!("{e:#}"))
    }

    fn batch_set(
        &mut self,
        self_: wasmtime::component::Resource<
            control_plane_component_bindings::tachyon::mesh::kv_partition::Table,
        >,
        entries: Vec<(String, Vec<u8>)>,
    ) -> std::result::Result<(), String> {
        let handle = wasmtime::component::Resource::<RedbTableResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        res.core_store
            .kv_partition_batch_set(&res.table_name, &entries)
            .map_err(|e| format!("{e:#}"))
    }

    fn get_range(
        &mut self,
        self_: wasmtime::component::Resource<
            control_plane_component_bindings::tachyon::mesh::kv_partition::Table,
        >,
        start_key: String,
        end_key: String,
        limit: u32,
        offset: u32,
    ) -> std::result::Result<Vec<(String, Vec<u8>)>, String> {
        let handle = wasmtime::component::Resource::<RedbTableResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        if let Some(ref denial) = res.scope_denial {
            return Err(denial.clone());
        }
        res.core_store
            .kv_partition_get_range(&res.table_name, &start_key, &end_key, limit, offset)
            .map_err(|e| format!("{e:#}"))
    }

    fn drop(
        &mut self,
        rep: wasmtime::component::Resource<
            control_plane_component_bindings::tachyon::mesh::kv_partition::Table,
        >,
    ) -> wasmtime::Result<()> {
        self.table
            .delete(wasmtime::component::Resource::<RedbTableResource>::new_own(
                rep.rep(),
            ))?;
        Ok(())
    }
}

/// Host state behind a `tachyon:accelerator/cpu` `token-stream` resource: the
/// receiving end of the channel the generation thread writes decoded fragments
/// into. `next` drains it; a closed channel marks the end of the stream.
/// A streaming generation in flight: the channel its decoded fragments arrive
/// on, and the slot its token counts land in when it finishes.
/// The WIT error every `tachyon:accelerator/cpu` generation function returns.
#[cfg(feature = "ai-inference")]
type WitGenerationError =
    accelerator_component_bindings::tachyon::accelerator::cpu::GenerationError;

/// Cross the host/guest boundary, keeping the upstream status intact. The
/// status is the whole point of the typed error: flattening to a message here
/// would put the guest back to guessing whether a failure is retryable.
#[cfg(feature = "ai-inference")]
fn wit_generation_error(error: ai_inference::GenerationError) -> WitGenerationError {
    WitGenerationError {
        message: error.message,
        upstream_status: error.upstream_status,
        class: error.class,
    }
}

#[cfg(feature = "ai-inference")]
fn wit_tool_call(
    call: ai_inference::ToolCall,
) -> accelerator_component_bindings::tachyon::accelerator::cpu::ToolCall {
    accelerator_component_bindings::tachyon::accelerator::cpu::ToolCall {
        id: call.id,
        name: call.name,
        arguments: call.arguments,
    }
}

/// How many decoded events may sit between the generation thread and the guest.
///
/// Small on purpose: this is a hand-off buffer that absorbs scheduling jitter,
/// not a place to store a response. A larger window only lets a fast producer
/// build a backlog the client has not asked for, which is the memory this bound
/// exists to cap.
#[cfg(feature = "ai-inference")]
const STREAM_CHANNEL_CAPACITY: usize = 64;

/// Bytes of decoded output allowed to sit between the generation thread and the
/// guest, per stream.
///
/// A count alone does not bound memory. One upstream SSE frame may approach
/// `MAX_SSE_FRAME_BYTES` (1 MiB), so 64 queued events could still hold ~64 MiB
/// per stream and, across the 32 concurrent streams the node admits, roughly
/// the same gigabytes the unbounded channel allowed. The two limits are kept
/// together because they bound different shapes of backlog: the count stops a
/// flood of tiny deltas, this stops a handful of enormous ones.
#[cfg(feature = "ai-inference")]
const STREAM_QUEUE_BUDGET_BYTES: usize = 256 * 1024;

/// The queued-bytes budget for one stream.
///
/// Producer-blocking rather than lossy: dropping a fragment would silently
/// truncate the answer, and the whole point of back-pressure here is that a
/// slow consumer slows the producer instead of costing it output.
#[cfg(feature = "ai-inference")]
#[derive(Default)]
struct StreamQueueBudget {
    state: Mutex<StreamQueueState>,
    drained: std::sync::Condvar,
}

#[cfg(feature = "ai-inference")]
#[derive(Default)]
struct StreamQueueState {
    queued: usize,
    consumer_gone: bool,
}

#[cfg(feature = "ai-inference")]
impl StreamQueueBudget {
    /// Block until `len` bytes fit, then charge them.
    ///
    /// `false` means the consumer has gone and the caller should stop — the
    /// same answer a failed `send` gives, available while merely waiting.
    fn reserve(&self, len: usize) -> bool {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            // A poisoned lock means the peer panicked; treat it as a departed
            // consumer rather than panicking this thread too.
            Err(_) => return false,
        };
        loop {
            if state.consumer_gone {
                return false;
            }
            // An empty queue always admits, whatever the size. Otherwise a
            // single event larger than the whole budget could never be sent and
            // the stream would hang instead of merely being slow.
            if state.queued == 0 || state.queued + len <= STREAM_QUEUE_BUDGET_BYTES {
                state.queued += len;
                return true;
            }
            state = match self.drained.wait(state) {
                Ok(state) => state,
                Err(_) => return false,
            };
        }
    }

    /// Give back the bytes of an event the guest has taken.
    fn release(&self, len: usize) {
        if let Ok(mut state) = self.state.lock() {
            state.queued = state.queued.saturating_sub(len);
        }
        self.drained.notify_all();
    }

    /// Wake a producer blocked on a budget nobody will drain again.
    fn consumer_gone(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.consumer_gone = true;
        }
        self.drained.notify_all();
    }
}

/// The generation thread's end of a guest stream.
///
/// Owns both cancellation signals: a failed `send` (the receiver is gone) and
/// the flag the resource's drop clears. They fire at the same moment — the
/// difference is that only the flag can be read without emitting.
#[cfg(feature = "ai-inference")]
struct GuestStreamSink<'a> {
    sender: &'a std::sync::mpsc::SyncSender<
        std::result::Result<StreamPayload, ai_inference::GenerationError>,
    >,
    consumer_alive: &'a Arc<std::sync::atomic::AtomicBool>,
    budget: &'a Arc<StreamQueueBudget>,
}

#[cfg(feature = "ai-inference")]
impl ai_inference::StreamSink for GuestStreamSink<'_> {
    fn emit(&mut self, event: ai_inference::StreamEvent<'_>) -> ai_inference::StreamControl {
        let payload = match event {
            ai_inference::StreamEvent::Content(text) => StreamPayload::Content(text.to_owned()),
            ai_inference::StreamEvent::Refusal(text) => StreamPayload::Refusal(text.to_owned()),
            ai_inference::StreamEvent::ToolCall(call) => StreamPayload::ToolCall(call),
        };
        // Charged before the send and refunded when the guest takes the event,
        // so the producer waits on the *bytes* outstanding rather than only on
        // the number of them.
        let charged = payload.queued_bytes();
        if !self.budget.reserve(charged) {
            return ai_inference::StreamControl::Stop;
        }
        match self.sender.send(Ok(payload)) {
            Ok(()) => ai_inference::StreamControl::Continue,
            Err(_) => {
                // Nothing was queued, so the reservation must not outlive the
                // failure — otherwise a later send would wait on bytes that
                // will never be drained.
                self.budget.release(charged);
                ai_inference::StreamControl::Stop
            }
        }
    }

    fn is_live(&mut self) -> bool {
        self.consumer_alive
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// One item the generation thread pushes down the stream channel. The owned
/// mirror of `ai_inference::StreamEvent`, which borrows its content so a
/// backend can emit a fragment without allocating when nobody needs to keep it.
#[cfg(feature = "ai-inference")]
enum StreamPayload {
    Content(String),
    Refusal(String),
    ToolCall(ai_inference::ToolCall),
}

#[cfg(feature = "ai-inference")]
impl StreamPayload {
    /// What this event costs while it waits in the channel. Approximate on
    /// purpose — it counts the heap-allocated text, which is the part that
    /// scales with the model's output and the only part worth bounding.
    fn queued_bytes(&self) -> usize {
        match self {
            Self::Content(text) | Self::Refusal(text) => text.len(),
            Self::ToolCall(call) => {
                call.name.len() + call.arguments.len() + call.id.as_ref().map_or(0, String::len)
            }
        }
    }
}

#[cfg(feature = "ai-inference")]
pub(crate) struct StreamedGeneration {
    receiver: std::sync::mpsc::Receiver<
        std::result::Result<StreamPayload, ai_inference::GenerationError>,
    >,
    outcome: Arc<Mutex<ai_inference::StreamOutcome>>,
    consumer_alive: Arc<std::sync::atomic::AtomicBool>,
    budget: Arc<StreamQueueBudget>,
}

#[cfg(feature = "ai-inference")]
pub(crate) struct HostTokenStream {
    receiver: std::sync::mpsc::Receiver<
        std::result::Result<StreamPayload, ai_inference::GenerationError>,
    >,
    outcome: Arc<Mutex<ai_inference::StreamOutcome>>,
    /// Refunded as each event is taken, and closed on drop so a producer
    /// waiting for room does not wait for a consumer that has gone.
    budget: Arc<StreamQueueBudget>,
    /// Cleared on drop, so a backend blocked between frames can see that this
    /// caller has gone without having to emit something first.
    consumer_alive: Arc<std::sync::atomic::AtomicBool>,
    /// Whether `next` has reported the end of the stream to this caller.
    ///
    /// The generation thread can finish while fragments are still queued, so
    /// the slot alone is not the contract the WIT promises: a caller polling
    /// `usage()` as its completion signal would see `Some` with decoded text
    /// still unread, stop reading, and truncate the response. Counts are
    /// withheld until the caller has actually observed EOF.
    saw_eof: bool,
}

#[cfg(feature = "ai-inference")]
impl accelerator_component_bindings::tachyon::accelerator::cpu::Host for ComponentHostState {
    fn load_model(&mut self, name: String) -> std::result::Result<u32, String> {
        self.load_accelerator_model(ai_inference::AcceleratorKind::Cpu, name)
    }

    fn compute(
        &mut self,
        model_id: u32,
        prompt: String,
    ) -> std::result::Result<String, WitGenerationError> {
        self.compute_accelerator_prompt(ai_inference::AcceleratorKind::Cpu, model_id, prompt)
            .map(|generation| generation.text)
            .map_err(wit_generation_error)
    }

    fn compute_detailed(
        &mut self,
        model_id: u32,
        prompt: String,
    ) -> std::result::Result<
        accelerator_component_bindings::tachyon::accelerator::cpu::Generation,
        WitGenerationError,
    > {
        let generation = self
            .compute_accelerator_prompt(ai_inference::AcceleratorKind::Cpu, model_id, prompt)
            .map_err(wit_generation_error)?;
        Ok(
            accelerator_component_bindings::tachyon::accelerator::cpu::Generation {
                text: generation.text,
                refusal: generation.refusal,
                finish_reason: generation.finish_reason,
                usage: generation.usage.map(|usage| {
                    accelerator_component_bindings::tachyon::accelerator::cpu::TokenUsage {
                        prompt_tokens: usage.prompt_tokens,
                        completion_tokens: usage.completion_tokens,
                    }
                }),
                tool_calls: generation
                    .tool_calls
                    .into_iter()
                    .map(wit_tool_call)
                    .collect(),
            },
        )
    }

    fn embed(
        &mut self,
        model_id: u32,
        input: String,
    ) -> std::result::Result<Vec<f32>, WitGenerationError> {
        self.embed_accelerator_input(ai_inference::AcceleratorKind::Cpu, model_id, input)
            .map_err(wit_generation_error)
    }

    fn compute_stream(
        &mut self,
        model_id: u32,
        prompt: String,
    ) -> std::result::Result<
        wasmtime::component::Resource<
            accelerator_component_bindings::tachyon::accelerator::cpu::TokenStream,
        >,
        WitGenerationError,
    > {
        let StreamedGeneration {
            receiver,
            outcome,
            consumer_alive,
            budget,
        } = self
            .stream_accelerator_prompt(ai_inference::AcceleratorKind::Cpu, model_id, prompt)
            .map_err(wit_generation_error)?;
        let handle = self
            .table
            .push(HostTokenStream {
                receiver,
                outcome,
                consumer_alive,
                budget,
                saw_eof: false,
            })
            .map_err(|error| {
                wit_generation_error(ai_inference::GenerationError::local(format!(
                    "failed to register token stream resource: {error}"
                )))
            })?;
        Ok(wasmtime::component::Resource::new_own(handle.rep()))
    }
}

#[cfg(feature = "ai-inference")]
impl accelerator_component_bindings::tachyon::accelerator::cpu::HostTokenStream
    for ComponentHostState
{
    fn next(
        &mut self,
        self_: wasmtime::component::Resource<
            accelerator_component_bindings::tachyon::accelerator::cpu::TokenStream,
        >,
    ) -> std::result::Result<
        Option<accelerator_component_bindings::tachyon::accelerator::cpu::StreamEvent>,
        WitGenerationError,
    > {
        let handle = wasmtime::component::Resource::<HostTokenStream>::new_borrow(self_.rep());
        let stream = self.table.get_mut(&handle).map_err(|error| {
            wit_generation_error(ai_inference::GenerationError::local(format!(
                "failed to access token stream resource: {error}"
            )))
        })?;
        // `recv` blocks until the next event; a disconnected channel means the
        // generation thread finished, so the stream is complete.
        let received = stream.receiver.recv();
        // Refund before answering, so the producer regains room as soon as the
        // bytes leave the queue rather than once the guest has finished with
        // them. An error payload carries no queued text and was never charged.
        if let Ok(Ok(payload)) = &received {
            stream.budget.release(payload.queued_bytes());
        }
        match received {
            Ok(Ok(StreamPayload::Content(text))) => Ok(Some(
                accelerator_component_bindings::tachyon::accelerator::cpu::StreamEvent::Content(
                    text,
                ),
            )),
            Ok(Ok(StreamPayload::Refusal(text))) => Ok(Some(
                accelerator_component_bindings::tachyon::accelerator::cpu::StreamEvent::Refusal(
                    text,
                ),
            )),
            Ok(Ok(StreamPayload::ToolCall(call))) => Ok(Some(
                accelerator_component_bindings::tachyon::accelerator::cpu::StreamEvent::ToolCall(
                    wit_tool_call(call),
                ),
            )),
            Ok(Err(error)) => Err(wit_generation_error(error)),
            Err(_) => {
                stream.saw_eof = true;
                Ok(None)
            }
        }
    }

    fn usage(
        &mut self,
        self_: wasmtime::component::Resource<
            accelerator_component_bindings::tachyon::accelerator::cpu::TokenStream,
        >,
    ) -> Option<accelerator_component_bindings::tachyon::accelerator::cpu::TokenUsage> {
        let handle = wasmtime::component::Resource::<HostTokenStream>::new_borrow(self_.rep());
        let stream = self.table.get(&handle).ok()?;
        if !stream.saw_eof {
            return None;
        }
        let reported = stream.outcome.lock().ok()?.usage?;
        Some(
            accelerator_component_bindings::tachyon::accelerator::cpu::TokenUsage {
                prompt_tokens: reported.prompt_tokens,
                completion_tokens: reported.completion_tokens,
            },
        )
    }

    fn finish_reason(
        &mut self,
        self_: wasmtime::component::Resource<
            accelerator_component_bindings::tachyon::accelerator::cpu::TokenStream,
        >,
    ) -> Option<String> {
        let handle = wasmtime::component::Resource::<HostTokenStream>::new_borrow(self_.rep());
        let stream = self.table.get(&handle).ok()?;
        // Withheld until the caller has observed EOF, for the same reason as
        // `usage`: the generation thread can finish while fragments are still
        // queued, and a caller polling this as its completion signal would stop
        // reading and truncate the response.
        if !stream.saw_eof {
            return None;
        }
        stream.outcome.lock().ok()?.finish_reason.clone()
    }

    fn drop(
        &mut self,
        rep: wasmtime::component::Resource<
            accelerator_component_bindings::tachyon::accelerator::cpu::TokenStream,
        >,
    ) -> wasmtime::Result<()> {
        let stream =
            self.table
                .delete(wasmtime::component::Resource::<HostTokenStream>::new_own(
                    rep.rep(),
                ))?;
        // Deleting the entry drops the receiver, which already makes the next
        // `send` fail. This says the same thing to a backend that is *waiting*
        // rather than sending — the case a slow upstream leaves it in, holding
        // an admission permit for a client that has gone.
        stream
            .consumer_alive
            .store(false, std::sync::atomic::Ordering::Relaxed);
        // And wake a producer parked on the byte budget. Dropping the receiver
        // unblocks a `send`, but a thread waiting for *room* is asleep on the
        // condvar, where a closed channel is not a signal it can see.
        stream.budget.consumer_gone();
        Ok(())
    }
}

#[cfg(feature = "ai-inference")]
impl accelerator_component_bindings::tachyon::accelerator::gpu::Host for ComponentHostState {
    fn load_model(&mut self, name: String) -> std::result::Result<u32, String> {
        self.load_accelerator_model(ai_inference::AcceleratorKind::Gpu, name)
    }

    fn compute(&mut self, model_id: u32, prompt: String) -> std::result::Result<String, String> {
        self.compute_accelerator_prompt(ai_inference::AcceleratorKind::Gpu, model_id, prompt)
            .map(|generation| generation.text)
            .map_err(|error| error.message)
    }
}

#[cfg(feature = "ai-inference")]
impl accelerator_component_bindings::tachyon::accelerator::npu::Host for ComponentHostState {
    fn load_model(&mut self, name: String) -> std::result::Result<u32, String> {
        self.load_accelerator_model(ai_inference::AcceleratorKind::Npu, name)
    }

    fn compute(&mut self, model_id: u32, prompt: String) -> std::result::Result<String, String> {
        self.compute_accelerator_prompt(ai_inference::AcceleratorKind::Npu, model_id, prompt)
            .map(|generation| generation.text)
            .map_err(|error| error.message)
    }
}

#[cfg(feature = "ai-inference")]
impl accelerator_component_bindings::tachyon::accelerator::tpu::Host for ComponentHostState {
    fn load_model(&mut self, name: String) -> std::result::Result<u32, String> {
        self.load_accelerator_model(ai_inference::AcceleratorKind::Tpu, name)
    }

    fn compute(&mut self, model_id: u32, prompt: String) -> std::result::Result<String, String> {
        self.compute_accelerator_prompt(ai_inference::AcceleratorKind::Tpu, model_id, prompt)
            .map(|generation| generation.text)
            .map_err(|error| error.message)
    }
}

#[cfg(feature = "websockets")]
impl websocket_component_bindings::tachyon::mesh::websocket::Host for ComponentHostState {}

#[cfg(feature = "websockets")]
impl websocket_component_bindings::tachyon::mesh::websocket::HostConnection for ComponentHostState {
    fn send(
        &mut self,
        self_: wasmtime::component::Resource<
            websocket_component_bindings::tachyon::mesh::websocket::Connection,
        >,
        frame: websocket_component_bindings::tachyon::mesh::websocket::Frame,
    ) -> std::result::Result<(), String> {
        let handle =
            wasmtime::component::Resource::<HostWebSocketConnection>::new_borrow(self_.rep());
        let connection = self
            .table
            .get(&handle)
            .map_err(|error| format!("failed to access WebSocket connection resource: {error}"))?;
        connection
            .outgoing
            .send(websocket_binding_frame_to_host_frame(frame))
            .map_err(|_| "WebSocket connection is closed".to_owned())
    }

    fn receive(
        &mut self,
        self_: wasmtime::component::Resource<
            websocket_component_bindings::tachyon::mesh::websocket::Connection,
        >,
    ) -> Option<websocket_component_bindings::tachyon::mesh::websocket::Frame> {
        let handle =
            wasmtime::component::Resource::<HostWebSocketConnection>::new_borrow(self_.rep());
        let connection = match self.table.get_mut(&handle) {
            Ok(connection) => connection,
            Err(_) => return None,
        };
        connection
            .incoming
            .recv()
            .ok()
            .map(host_frame_to_websocket_binding_frame)
    }

    fn drop(
        &mut self,
        rep: wasmtime::component::Resource<
            websocket_component_bindings::tachyon::mesh::websocket::Connection,
        >,
    ) -> wasmtime::Result<()> {
        self.table
            .delete(wasmtime::component::Resource::<HostWebSocketConnection>::new_own(rep.rep()))?;
        Ok(())
    }
}

impl system_component_bindings::tachyon::mesh::telemetry_reader::Host for ComponentHostState {
    fn get_metrics(
        &mut self,
    ) -> system_component_bindings::tachyon::mesh::telemetry_reader::MetricsSnapshot {
        let TelemetrySnapshot {
            total_requests,
            completed_requests,
            error_requests,
            active_requests,
            dropped_events,
            last_status,
            total_duration_us,
            total_wasm_duration_us,
            total_host_overhead_us,
        } = telemetry::snapshot(&self.telemetry);
        let control_plane = control_plane_snapshot(
            &self.telemetry,
            self.host_load.as_ref(),
            self.concurrency_limits.as_ref(),
            &self.runtime_config,
            self.host_capabilities,
            self.accelerator_queue_loads(),
        );
        let l4 = self.bridge_manager.telemetry_snapshot();
        let hot_models = self.hot_model_aliases();

        system_component_bindings::tachyon::mesh::telemetry_reader::MetricsSnapshot {
            total_requests,
            completed_requests,
            error_requests,
            active_requests,
            cpu_pressure: control_plane.cpu_pressure,
            ram_pressure: control_plane.ram_pressure,
            active_instances: control_plane.active_instances,
            allocated_memory_pages: control_plane.allocated_memory_pages,
            capability_mask: control_plane.capability_mask,
            capabilities: control_plane.capabilities,
            active_l4_relays: l4.active_relays,
            l4_throughput_bytes_per_sec: l4.throughput_bytes_per_sec,
            l4_load_score: l4.load_score,
            advertise_ip: effective_advertise_ip(&self.runtime_config),
            cpu_rt_load: control_plane.cpu_rt_load,
            cpu_standard_load: control_plane.cpu_standard_load,
            cpu_batch_load: control_plane.cpu_batch_load,
            gpu_rt_load: control_plane.gpu_rt_load,
            gpu_standard_load: control_plane.gpu_standard_load,
            gpu_batch_load: control_plane.gpu_batch_load,
            npu_rt_load: control_plane.npu_rt_load,
            npu_standard_load: control_plane.npu_standard_load,
            npu_batch_load: control_plane.npu_batch_load,
            tpu_rt_load: control_plane.tpu_rt_load,
            tpu_standard_load: control_plane.tpu_standard_load,
            tpu_batch_load: control_plane.tpu_batch_load,
            network_rt_load: control_plane.network_rt_load,
            network_standard_load: control_plane.network_standard_load,
            network_batch_load: control_plane.network_batch_load,
            hot_models,
            dropped_events,
            last_status,
            total_duration_us,
            total_wasm_duration_us,
            total_host_overhead_us,
        }
    }
}

impl control_plane_component_bindings::tachyon::mesh::telemetry_reader::Host
    for ComponentHostState
{
    fn get_metrics(
        &mut self,
    ) -> control_plane_component_bindings::tachyon::mesh::telemetry_reader::MetricsSnapshot {
        let snapshot =
            <Self as system_component_bindings::tachyon::mesh::telemetry_reader::Host>::get_metrics(
                self,
            );
        control_plane_component_bindings::tachyon::mesh::telemetry_reader::MetricsSnapshot {
            total_requests: snapshot.total_requests,
            completed_requests: snapshot.completed_requests,
            error_requests: snapshot.error_requests,
            active_requests: snapshot.active_requests,
            cpu_pressure: snapshot.cpu_pressure,
            ram_pressure: snapshot.ram_pressure,
            active_instances: snapshot.active_instances,
            allocated_memory_pages: snapshot.allocated_memory_pages,
            capability_mask: snapshot.capability_mask,
            capabilities: snapshot.capabilities,
            active_l4_relays: snapshot.active_l4_relays,
            l4_throughput_bytes_per_sec: snapshot.l4_throughput_bytes_per_sec,
            l4_load_score: snapshot.l4_load_score,
            advertise_ip: snapshot.advertise_ip,
            cpu_rt_load: snapshot.cpu_rt_load,
            cpu_standard_load: snapshot.cpu_standard_load,
            cpu_batch_load: snapshot.cpu_batch_load,
            gpu_rt_load: snapshot.gpu_rt_load,
            gpu_standard_load: snapshot.gpu_standard_load,
            gpu_batch_load: snapshot.gpu_batch_load,
            npu_rt_load: snapshot.npu_rt_load,
            npu_standard_load: snapshot.npu_standard_load,
            npu_batch_load: snapshot.npu_batch_load,
            tpu_rt_load: snapshot.tpu_rt_load,
            tpu_standard_load: snapshot.tpu_standard_load,
            tpu_batch_load: snapshot.tpu_batch_load,
            network_rt_load: snapshot.network_rt_load,
            network_standard_load: snapshot.network_standard_load,
            network_batch_load: snapshot.network_batch_load,
            hot_models: snapshot.hot_models,
            dropped_events: snapshot.dropped_events,
            last_status: snapshot.last_status,
            total_duration_us: snapshot.total_duration_us,
            total_wasm_duration_us: snapshot.total_wasm_duration_us,
            total_host_overhead_us: snapshot.total_host_overhead_us,
        }
    }
}

impl background_component_bindings::tachyon::mesh::telemetry_reader::Host for ComponentHostState {
    fn get_metrics(
        &mut self,
    ) -> background_component_bindings::tachyon::mesh::telemetry_reader::MetricsSnapshot {
        let snapshot =
            <Self as system_component_bindings::tachyon::mesh::telemetry_reader::Host>::get_metrics(
                self,
            );
        background_component_bindings::tachyon::mesh::telemetry_reader::MetricsSnapshot {
            total_requests: snapshot.total_requests,
            completed_requests: snapshot.completed_requests,
            error_requests: snapshot.error_requests,
            active_requests: snapshot.active_requests,
            cpu_pressure: snapshot.cpu_pressure,
            ram_pressure: snapshot.ram_pressure,
            active_instances: snapshot.active_instances,
            allocated_memory_pages: snapshot.allocated_memory_pages,
            capability_mask: snapshot.capability_mask,
            capabilities: snapshot.capabilities,
            active_l4_relays: snapshot.active_l4_relays,
            l4_throughput_bytes_per_sec: snapshot.l4_throughput_bytes_per_sec,
            l4_load_score: snapshot.l4_load_score,
            advertise_ip: snapshot.advertise_ip,
            cpu_rt_load: snapshot.cpu_rt_load,
            cpu_standard_load: snapshot.cpu_standard_load,
            cpu_batch_load: snapshot.cpu_batch_load,
            gpu_rt_load: snapshot.gpu_rt_load,
            gpu_standard_load: snapshot.gpu_standard_load,
            gpu_batch_load: snapshot.gpu_batch_load,
            npu_rt_load: snapshot.npu_rt_load,
            npu_standard_load: snapshot.npu_standard_load,
            npu_batch_load: snapshot.npu_batch_load,
            tpu_rt_load: snapshot.tpu_rt_load,
            tpu_standard_load: snapshot.tpu_standard_load,
            tpu_batch_load: snapshot.tpu_batch_load,
            network_rt_load: snapshot.network_rt_load,
            network_standard_load: snapshot.network_standard_load,
            network_batch_load: snapshot.network_batch_load,
            hot_models: snapshot.hot_models,
            dropped_events: snapshot.dropped_events,
            last_status: snapshot.last_status,
            total_duration_us: snapshot.total_duration_us,
            total_wasm_duration_us: snapshot.total_wasm_duration_us,
            total_host_overhead_us: snapshot.total_host_overhead_us,
        }
    }
}

impl system_component_bindings::tachyon::mesh::scaling_metrics::Host for ComponentHostState {
    fn get_pending_queue_size(&mut self, route_path: String) -> u32 {
        self.pending_queue_size(&route_path)
    }
}

impl component_bindings::tachyon::mesh::custom_metrics::Host for ComponentHostState {
    fn push(
        &mut self,
        data: component_bindings::tachyon::mesh::custom_metrics::Metric,
    ) -> std::result::Result<(), String> {
        push_custom_metric(CustomMetric {
            name: data.name,
            value: data.value,
            metric_type: match data.kind {
                component_bindings::tachyon::mesh::custom_metrics::MetricType::Counter => {
                    CustomMetricType::Counter
                }
                component_bindings::tachyon::mesh::custom_metrics::MetricType::Gauge => {
                    CustomMetricType::Gauge
                }
                component_bindings::tachyon::mesh::custom_metrics::MetricType::Histogram => {
                    CustomMetricType::Histogram
                }
            },
            tags: data.tags,
        })
    }
}

impl system_component_bindings::tachyon::mesh::custom_metrics::Host for ComponentHostState {
    fn push(
        &mut self,
        data: system_component_bindings::tachyon::mesh::custom_metrics::Metric,
    ) -> std::result::Result<(), String> {
        push_custom_metric(CustomMetric {
            name: data.name,
            value: data.value,
            metric_type: match data.kind {
                system_component_bindings::tachyon::mesh::custom_metrics::MetricType::Counter => {
                    CustomMetricType::Counter
                }
                system_component_bindings::tachyon::mesh::custom_metrics::MetricType::Gauge => {
                    CustomMetricType::Gauge
                }
                system_component_bindings::tachyon::mesh::custom_metrics::MetricType::Histogram => {
                    CustomMetricType::Histogram
                }
            },
            tags: data.tags,
        })
    }
}

impl background_component_bindings::tachyon::mesh::custom_metrics::Host for ComponentHostState {
    fn push(
        &mut self,
        data: background_component_bindings::tachyon::mesh::custom_metrics::Metric,
    ) -> std::result::Result<(), String> {
        push_custom_metric(CustomMetric {
            name: data.name,
            value: data.value,
            metric_type: match data.kind {
                background_component_bindings::tachyon::mesh::custom_metrics::MetricType::Counter => {
                    CustomMetricType::Counter
                }
                background_component_bindings::tachyon::mesh::custom_metrics::MetricType::Gauge => {
                    CustomMetricType::Gauge
                }
                background_component_bindings::tachyon::mesh::custom_metrics::MetricType::Histogram => {
                    CustomMetricType::Histogram
                }
            },
            tags: data.tags,
        })
    }
}

impl control_plane_component_bindings::tachyon::mesh::custom_metrics::Host for ComponentHostState {
    fn push(
        &mut self,
        data: control_plane_component_bindings::tachyon::mesh::custom_metrics::Metric,
    ) -> std::result::Result<(), String> {
        push_custom_metric(CustomMetric {
            name: data.name,
            value: data.value,
            metric_type: match data.kind {
                control_plane_component_bindings::tachyon::mesh::custom_metrics::MetricType::Counter => {
                    CustomMetricType::Counter
                }
                control_plane_component_bindings::tachyon::mesh::custom_metrics::MetricType::Gauge => {
                    CustomMetricType::Gauge
                }
                control_plane_component_bindings::tachyon::mesh::custom_metrics::MetricType::Histogram => {
                    CustomMetricType::Histogram
                }
            },
            tags: data.tags,
        })
    }
}

impl control_plane_component_bindings::tachyon::mesh::scaling_metrics::Host for ComponentHostState {
    fn get_pending_queue_size(&mut self, route_path: String) -> u32 {
        self.pending_queue_size(&route_path)
    }
}

impl system_component_bindings::tachyon::mesh::storage_broker::Host for ComponentHostState {
    fn enqueue_write(
        &mut self,
        path: String,
        mode: system_component_bindings::tachyon::mesh::storage_broker::WriteMode,
        body: Vec<u8>,
    ) -> std::result::Result<(), String> {
        if !self.scopes.check_storage(&path) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Storage,
                &self.route_path,
            );
            return Err(format!(
                "permission denied: storage path `{path}` not granted by deployment scopes"
            ));
        }
        let mode = match mode {
            system_component_bindings::tachyon::mesh::storage_broker::WriteMode::Overwrite => {
                StorageWriteMode::Overwrite
            }
            system_component_bindings::tachyon::mesh::storage_broker::WriteMode::Append => {
                StorageWriteMode::Append
            }
        };
        let (route, resolved) = authorize_storage_broker_write(
            &self.runtime_config,
            &self.request_headers,
            self.host_identity.as_ref(),
            &path,
        )?;

        self.storage_broker.enqueue_write_target(
            route.path,
            route.sync_to_cloud,
            resolved,
            mode,
            body,
        )
    }

    fn snapshot_volume(
        &mut self,
        volume_id: String,
        source_path: String,
        snapshot_path: String,
    ) -> std::result::Result<(), String> {
        if !self.scopes.check_storage(&volume_id) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Storage,
                &self.route_path,
            );
            return Err(format!(
                "permission denied: storage volume `{volume_id}` not granted by deployment scopes"
            ));
        }
        let source_path = parse_storage_broker_host_path(&source_path, "source_path")?;
        let snapshot_path = parse_storage_broker_host_path(&snapshot_path, "snapshot_path")?;
        drop(self.storage_broker.enqueue_snapshot(
            volume_id,
            &source_path,
            &source_path,
            &snapshot_path,
        )?);
        Ok(())
    }

    fn restore_volume(
        &mut self,
        volume_id: String,
        snapshot_path: String,
        destination_path: String,
    ) -> std::result::Result<(), String> {
        if !self.scopes.check_storage(&volume_id) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Storage,
                &self.route_path,
            );
            return Err(format!(
                "permission denied: storage volume `{volume_id}` not granted by deployment scopes"
            ));
        }
        let snapshot_path = parse_storage_broker_host_path(&snapshot_path, "snapshot_path")?;
        let destination_path =
            parse_storage_broker_host_path(&destination_path, "destination_path")?;
        drop(self.storage_broker.enqueue_restore(
            volume_id,
            &destination_path,
            &snapshot_path,
            &destination_path,
        )?);
        Ok(())
    }
}

impl background_component_bindings::tachyon::mesh::scaling_metrics::Host for ComponentHostState {
    fn get_pending_queue_size(&mut self, route_path: String) -> u32 {
        self.pending_queue_size(&route_path)
    }
}

impl control_plane_component_bindings::tachyon::mesh::routing_control::Host for ComponentHostState {
    fn update_target(
        &mut self,
        route_path: String,
        destination: String,
    ) -> std::result::Result<(), String> {
        if !self.scopes.check_routing(&route_path, &destination) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Routing,
                &self.route_path,
            );
            return Err(format!(
                "permission denied: routing `{route_path}` → `{destination}` not granted by deployment scopes"
            ));
        }
        update_control_plane_route_override(
            self.route_overrides.as_ref(),
            &self.peer_capabilities,
            &route_path,
            &destination,
        )
    }
}

impl background_component_bindings::tachyon::mesh::routing_control::Host for ComponentHostState {
    fn update_target(
        &mut self,
        route_path: String,
        destination: String,
    ) -> std::result::Result<(), String> {
        if !self.scopes.check_routing(&route_path, &destination) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Routing,
                &self.route_path,
            );
            return Err(format!(
                "permission denied: routing `{route_path}` → `{destination}` not granted by deployment scopes"
            ));
        }
        update_control_plane_route_override(
            self.route_overrides.as_ref(),
            &self.peer_capabilities,
            &route_path,
            &destination,
        )
    }
}

// Must match the casing `guest-openai` uses to read the `ai-models-registry`
// table (`#[serde(rename_all = "camelCase")]`); otherwise its `list_models`
// drops the row on a deserialize miss and the model never appears in
// `GET /ai/v1/models`. See the matching note on `RegistryModelInfo` in
// `system_storage.rs`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelRegistryRecord<'a> {
    alias: &'a str,
    engine: &'a str,
    vram_required_mb: u64,
    status: &'a str,
    model_path: &'a str,
    /// See the matching field on `RegistryModelInfo` in `system_storage.rs`:
    /// `guest-openai` reads this to pick a tool-call parser instead of
    /// pattern-matching the alias.
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_parser: Option<&'a str>,
}

impl system_component_bindings::tachyon::mesh::model_events::Host for ComponentHostState {
    fn publish_model_uploaded(
        &mut self,
        event: system_component_bindings::tachyon::mesh::model_events::ModelUploaded,
    ) -> std::result::Result<(), String> {
        if event.alias.trim().is_empty() {
            return Err("model upload event alias must not be empty".to_owned());
        }
        if event.engine.trim().is_empty() {
            return Err("model upload event engine must not be empty".to_owned());
        }
        let record = ModelRegistryRecord {
            alias: &event.alias,
            engine: &event.engine,
            vram_required_mb: 0,
            status: "available",
            model_path: &event.model_path,
            tool_call_parser: crate::system_storage::binding_tool_call_parser(&event.model_path),
        };
        let value = serde_json::to_vec(&record)
            .map_err(|error| format!("failed to encode model registry entry: {error}"))?;
        // NOTE: unlike the storage-proxy path (`StorageComponentState`), this
        // general component-host path does NOT flush the model to S3. If this log
        // fires during an upload, that is why the bucket stays empty.
        tracing::info!(
            alias = %event.alias,
            engine = %event.engine,
            model_path = %event.model_path,
            "publishing uploaded model to registry via ComponentHostState (no S3 flush on this path)"
        );
        // Through the shared writer, not a bare `set`: the alias-ownership rule
        // has to hold on every path into the registry, and this one is reached
        // by the ordinary component host rather than the storage proxy.
        crate::system_storage::write_uploaded_model_row(
            &self.storage_broker.core_store,
            &event.alias,
            value,
        )
    }
}

impl component_bindings::tachyon::mesh::outbound_http::Host for ComponentHostState {
    fn send_request(
        &mut self,
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> std::result::Result<component_bindings::tachyon::mesh::outbound_http::Response, String>
    {
        let response =
            <Self as background_component_bindings::tachyon::mesh::outbound_http::Host>::send_request(
                self, method, url, headers, body,
            )?;
        Ok(component_bindings::tachyon::mesh::outbound_http::Response {
            status: response.status,
            headers: response.headers,
            body: response.body,
        })
    }
}

impl background_component_bindings::tachyon::mesh::outbound_http::Host for ComponentHostState {
    fn send_request(
        &mut self,
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> std::result::Result<
        background_component_bindings::tachyon::mesh::outbound_http::Response,
        String,
    > {
        // Scope check on scheme://host/path (query string stripped per spec D5).
        let url_without_query = crate::host_core::scoping::strip_url_query(&url);
        if !self.scopes.check_http(url_without_query) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Http,
                &self.route_path,
            );
            return Err(format!(
                "permission denied: outbound HTTP URL `{url_without_query}` not granted by deployment scopes"
            ));
        }
        let method = reqwest::Method::from_bytes(method.trim().as_bytes())
            .map_err(|error| format!("invalid outbound HTTP method `{method}`: {error}"))?;
        let route_registry = RouteRegistry::build(&self.runtime_config)
            .map_err(|error| format!("failed to build sealed route registry: {error:#}"))?;
        let caller_route = self
            .runtime_config
            .sealed_route(&self.route_path)
            .ok_or_else(|| {
                format!(
                    "sealed caller route `{}` is not present in `integrity.lock`",
                    self.route_path
                )
            })?;
        let resolved_target = resolve_outbound_http_target(
            &self.runtime_config,
            &route_registry,
            caller_route,
            &method,
            &url,
        )?;
        let outbound_headers = filtered_outbound_http_headers(
            headers,
            &self.propagated_headers,
            &resolved_target.kind,
        );
        let local_fallback_reason = if resolved_target.kind.is_internal() {
            match self.try_send_in_process_outbound_request(
                &resolved_target.url,
                &method,
                &outbound_headers,
                &body,
            )? {
                LocalMeshOutboundAttempt::Handled(response) => return Ok(response),
                LocalMeshOutboundAttempt::Fallback(reason) => Some(reason),
            }
        } else {
            None
        };

        let url = rewrite_outbound_http_url(&resolved_target.url, &self.runtime_config);

        tracing::info!(
            method = %method,
            url = %url,
            bytes = body.len(),
            "autoscaling guest sending outbound HTTP request"
        );

        let (headers, body) = inject_outbound_secrets(
            outbound_headers,
            body,
            outbound_target_host(&url).as_deref(),
        );
        #[cfg(unix)]
        if resolved_target.kind.is_internal() {
            if let Some(context) = self.local_mesh_dispatch.as_ref() {
                if let Some(response) = send_blocking_uds_fast_path_request(
                    context.state.uds_fast_path.as_ref(),
                    &url,
                    &method,
                    &headers,
                    &body,
                ) {
                    return Ok(response);
                }
            }
        }

        let mut request = self.outbound_http_client.request(method, &url);
        for (name, value) in &headers {
            request = request.header(name, value);
        }
        let tcp_started_at = Instant::now();
        let response = request
            .body(body)
            .send()
            .map_err(|error| format!("failed to send outbound HTTP request to `{url}`: {error}"))?;
        let status = response.status().as_u16();
        let response_headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_owned(),
                    value.to_str().unwrap_or_default().to_owned(),
                )
            })
            .collect::<Vec<_>>();
        let body = response
            .bytes()
            .map_err(|error| {
                format!("failed to read outbound HTTP response body from `{url}`: {error}")
            })?
            .to_vec();
        if resolved_target.kind.is_internal() {
            record_mesh_dispatch(
                MeshDispatchMode::Tcp,
                local_fallback_reason.unwrap_or(MeshDispatchReason::Remote),
                tcp_started_at.elapsed(),
            );
        }

        Ok(
            background_component_bindings::tachyon::mesh::outbound_http::Response {
                status,
                headers: response_headers,
                body,
            },
        )
    }
}

#[cfg(unix)]
pub(crate) fn send_blocking_uds_fast_path_request(
    uds_fast_path: &UdsFastPathRegistry,
    url: &str,
    method: &reqwest::Method,
    headers: &[(String, String)],
    body: &[u8],
) -> Option<background_component_bindings::tachyon::mesh::outbound_http::Response> {
    let started_at = Instant::now();
    let peer = match uds_fast_path.discover_peer_for_url(url) {
        Some(peer) => peer,
        None => {
            record_mesh_dispatch(
                MeshDispatchMode::Uds,
                MeshDispatchReason::Remote,
                started_at.elapsed(),
            );
            return None;
        }
    };
    let client = match uds_fast_path.blocking_client_for_peer(&peer) {
        Ok(client) => client,
        Err(error) => {
            tracing::debug!(
                socket = %peer.socket_path.display(),
                url = %url,
                "UDS fast-path client unavailable, falling back to TCP: {error:#}"
            );
            record_mesh_dispatch(
                MeshDispatchMode::Uds,
                MeshDispatchReason::Remote,
                started_at.elapsed(),
            );
            return None;
        }
    };

    let mut request = client.request(method.clone(), url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = match request.body(body.to_vec()).send() {
        Ok(response) => response,
        Err(error) => {
            uds_fast_path.note_connect_failure(&peer);
            tracing::debug!(
                socket = %peer.socket_path.display(),
                url = %url,
                "UDS fast-path unavailable, falling back to TCP: {error}"
            );
            record_mesh_dispatch(
                MeshDispatchMode::Uds,
                MeshDispatchReason::Remote,
                started_at.elapsed(),
            );
            return None;
        }
    };
    let status = response.status().as_u16();
    let response_headers = response
        .headers()
        .iter()
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                value.to_str().unwrap_or_default().to_owned(),
            )
        })
        .collect::<Vec<_>>();
    let body = match response.bytes() {
        Ok(body) => body.to_vec(),
        Err(error) => {
            tracing::debug!(
                socket = %peer.socket_path.display(),
                url = %url,
                "failed to read UDS fast-path response body, falling back to TCP: {error}"
            );
            record_mesh_dispatch(
                MeshDispatchMode::Uds,
                MeshDispatchReason::Remote,
                started_at.elapsed(),
            );
            return None;
        }
    };

    record_mesh_dispatch(
        MeshDispatchMode::Uds,
        MeshDispatchReason::Ok,
        started_at.elapsed(),
    );

    Some(
        background_component_bindings::tachyon::mesh::outbound_http::Response {
            status,
            headers: response_headers,
            body,
        },
    )
}

impl ComponentHostState {
    fn try_send_in_process_outbound_request(
        &self,
        resolved_url: &str,
        method: &reqwest::Method,
        headers: &[(String, String)],
        body: &[u8],
    ) -> std::result::Result<LocalMeshOutboundAttempt, String> {
        let Some(context) = self.local_mesh_dispatch.as_ref() else {
            return Ok(LocalMeshOutboundAttempt::Fallback(
                MeshDispatchReason::Remote,
            ));
        };
        let method = Method::from_bytes(method.as_str().as_bytes())
            .map_err(|error| format!("invalid local outbound HTTP method `{method}`: {error}"))?;
        let header_map = guest_fields_to_header_map(&headers.to_vec(), "outbound HTTP headers")?;
        let response = context.handle.block_on(try_dispatch_local_mesh_request(
            &context.state,
            &context.runtime,
            resolved_url,
            &method,
            &header_map,
            Bytes::copy_from_slice(body),
            Vec::new(),
            resolve_incoming_hop_limit(&self.request_headers)
                .unwrap_or(HopLimit(DEFAULT_HOP_LIMIT)),
        ))?;

        Ok(match response {
            LocalMeshDispatchAttempt::Handled(response) => LocalMeshOutboundAttempt::Handled(
                background_component_bindings::tachyon::mesh::outbound_http::Response {
                    status: response.status.as_u16(),
                    headers: response.headers,
                    body: response.body.to_vec(),
                },
            ),
            LocalMeshDispatchAttempt::Fallback(reason) => {
                LocalMeshOutboundAttempt::Fallback(reason)
            }
        })
    }
}

enum LocalMeshOutboundAttempt {
    Handled(background_component_bindings::tachyon::mesh::outbound_http::Response),
    Fallback(MeshDispatchReason),
}

impl background_component_bindings::tachyon::mesh::outbox_store::Host for ComponentHostState {
    fn claim_events(
        &mut self,
        db_url: String,
        table: String,
        max_events: u32,
    ) -> std::result::Result<
        Vec<background_component_bindings::tachyon::mesh::outbox_store::OutboxEvent>,
        String,
    > {
        if !self.scopes.check_outbox(&db_url, &table) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Outbox,
                &self.route_path,
            );
            return Err(format!(
                "permission denied: outbox `{db_url}/{table}` not granted by deployment scopes"
            ));
        }
        data_events::claim_events(
            self.storage_broker.core_store.as_ref(),
            &db_url,
            &table,
            max_events,
        )
        .map(|events| {
            events
                .into_iter()
                .map(|event| {
                    background_component_bindings::tachyon::mesh::outbox_store::OutboxEvent {
                        id: event.id,
                        content_type: event.content_type,
                        body: event.body,
                    }
                })
                .collect()
        })
        .map_err(|error| format!("failed to claim outbox events: {error}"))
    }

    fn ack_event(
        &mut self,
        db_url: String,
        table: String,
        id: String,
    ) -> std::result::Result<(), String> {
        if !self.scopes.check_outbox(&db_url, &table) {
            self.scope_denials.increment_with_deployment(
                crate::host_core::scoping::ScopeCategory::Outbox,
                &self.route_path,
            );
            return Err(format!(
                "permission denied: outbox `{db_url}/{table}` not granted by deployment scopes"
            ));
        }
        data_events::ack_event(
            self.storage_broker.core_store.as_ref(),
            &db_url,
            &table,
            &id,
        )
        .map_err(|error| format!("failed to ack outbox event `{id}`: {error}"))
    }
}

impl control_plane_component_bindings::tachyon::mesh::outbound_http::Host for ComponentHostState {
    fn send_request(
        &mut self,
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> std::result::Result<
        control_plane_component_bindings::tachyon::mesh::outbound_http::Response,
        String,
    > {
        let response =
            <Self as background_component_bindings::tachyon::mesh::outbound_http::Host>::send_request(
                self, method, url, headers, body,
            )?;
        Ok(
            control_plane_component_bindings::tachyon::mesh::outbound_http::Response {
                status: response.status,
                headers: response.headers,
                body: response.body,
            },
        )
    }
}

#[cfg(feature = "websockets")]
impl websocket_component_bindings::tachyon::mesh::outbound_http::Host for ComponentHostState {
    fn send_request(
        &mut self,
        method: String,
        url: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> std::result::Result<
        websocket_component_bindings::tachyon::mesh::outbound_http::Response,
        String,
    > {
        let response =
            <Self as background_component_bindings::tachyon::mesh::outbound_http::Host>::send_request(
                self, method, url, headers, body,
            )?;
        Ok(
            websocket_component_bindings::tachyon::mesh::outbound_http::Response {
                status: response.status,
                headers: response.headers,
                body: response.body,
            },
        )
    }
}

pub(crate) fn rewrite_outbound_http_url(url: &str, runtime_config: &IntegrityConfig) -> String {
    if let Some(path) = url.strip_prefix("http://mesh") {
        let host = runtime_config
            .host_address
            .parse::<SocketAddr>()
            .map(|address| SocketAddr::new(loopback_ip_for(address.ip()), address.port()))
            .map(|address| address.to_string())
            .unwrap_or_else(|_| runtime_config.host_address.clone());
        return format!("http://{host}{path}");
    }

    let Some(mock_base_url) = std::env::var(MOCK_K8S_URL_ENV)
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_owned())
        .filter(|value| !value.is_empty())
    else {
        return url.to_owned();
    };

    if let Some(suffix) = url.strip_prefix(KUBERNETES_SERVICE_BASE_URL) {
        format!("{mock_base_url}{suffix}")
    } else {
        url.to_owned()
    }
}

pub(crate) fn filtered_outbound_http_headers(
    headers: Vec<(String, String)>,
    propagated_headers: &[PropagatedHeader],
    target_kind: &OutboundTargetKind,
) -> Vec<(String, String)> {
    let mut filtered = headers;
    if target_kind.is_internal() {
        filtered.extend(
            propagated_headers
                .iter()
                .map(|header| (header.name.clone(), header.value.clone())),
        );
        return filtered;
    }

    filtered.retain(|(name, _)| allow_external_outbound_header(name));
    filtered
}

pub(crate) fn inject_outbound_secrets(
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    target_host: Option<&str>,
) -> (Vec<(String, String)>, Vec<u8>) {
    let Some(target_host) = target_host else {
        return (headers, body);
    };
    let headers = headers
        .into_iter()
        .map(|(name, value)| {
            (
                name,
                rewrite_secret_placeholders(&value, target_host, "header value"),
            )
        })
        .collect();
    let body = rewrite_secret_placeholders_in_body(body, target_host);
    (headers, body)
}

fn rewrite_secret_placeholders_in_body(body: Vec<u8>, target_host: &str) -> Vec<u8> {
    if !body
        .windows(store::secrets::SECRET_PLACEHOLDER_PREFIX.len())
        .any(|window| window == store::secrets::SECRET_PLACEHOLDER_PREFIX.as_bytes())
    {
        return body;
    }

    match String::from_utf8(body) {
        Ok(value) => rewrite_secret_placeholders(&value, target_host, "request body").into_bytes(),
        Err(error) => error.into_bytes(),
    }
}

fn rewrite_secret_placeholders(value: &str, target_host: &str, field: &str) -> String {
    let mut rewritten = String::with_capacity(value.len());
    let mut remaining = value;

    while let Some(index) = remaining.find(store::secrets::SECRET_PLACEHOLDER_PREFIX) {
        rewritten.push_str(&remaining[..index]);
        let candidate = &remaining[index..];
        let Some(token) = store::secrets::placeholder_token_at(candidate) else {
            rewritten.push_str(store::secrets::SECRET_PLACEHOLDER_PREFIX);
            remaining = &candidate[store::secrets::SECRET_PLACEHOLDER_PREFIX.len()..];
            continue;
        };

        match store::secrets::resolve_secret(token, target_host) {
            Ok(secret) => rewritten.push_str(&secret),
            Err(error) => {
                tracing::warn!(
                    target_host,
                    field,
                    reason = %error,
                    "leaving unresolved outbound secret placeholder"
                );
                rewritten.push_str(token);
            }
        }
        remaining = &candidate[token.len()..];
    }

    rewritten.push_str(remaining);
    rewritten
}

fn outbound_target_host(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
}

pub(crate) fn allow_external_outbound_header(name: &str) -> bool {
    ![
        HOP_LIMIT_HEADER,
        COHORT_HEADER,
        TACHYON_COHORT_HEADER,
        TACHYON_IDENTITY_HEADER,
        TACHYON_ORIGINAL_ROUTE_HEADER,
        TACHYON_BUFFER_REPLAY_HEADER,
        "connection",
        "content-length",
        "host",
        "keep-alive",
        "proxy-connection",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ]
    .iter()
    .any(|forbidden| name.eq_ignore_ascii_case(forbidden))
}

pub(crate) fn loopback_ip_for(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
    }
}

// ── tachyon:mesh/response-body ────────────────────────────────────────────────

impl component_bindings::tachyon::mesh::response_body::Host for ComponentHostState {
    fn get_streaming_response(
        &mut self,
    ) -> std::result::Result<
        wasmtime::component::Resource<
            component_bindings::tachyon::mesh::response_body::StreamingResponse,
        >,
        String,
    > {
        let slot = self
            .streaming_body
            .take()
            .ok_or_else(|| "no streaming response available for this request".to_owned())?;
        let resource = HostStreamingResponseResource {
            headers_tx: Some(slot.headers_tx),
            chunk_tx: slot.chunk_tx,
            #[cfg(feature = "ai-inference")]
            consumer_alive: Arc::clone(&slot.consumer_alive),
        };
        let owned = self
            .table
            .push(resource)
            .expect("resource table push failed");
        Ok(wasmtime::component::Resource::new_own(owned.rep()))
    }
}

impl component_bindings::tachyon::mesh::response_body::HostStreamingResponse
    for ComponentHostState
{
    fn begin(
        &mut self,
        self_: wasmtime::component::Resource<
            component_bindings::tachyon::mesh::response_body::StreamingResponse,
        >,
        status: u16,
        headers: Vec<(String, String)>,
    ) -> std::result::Result<(), String> {
        let handle =
            wasmtime::component::Resource::<HostStreamingResponseResource>::new_borrow(self_.rep());
        let res = self.table.get_mut(&handle).map_err(|e| format!("{e:#}"))?;
        let tx = res
            .headers_tx
            .take()
            .ok_or_else(|| "begin() called more than once on streaming-response".to_owned())?;
        let sc = StatusCode::from_u16(status)
            .map_err(|e| format!("invalid status code {status}: {e}"))?;
        let _ = tx.send((sc, headers));
        Ok(())
    }

    fn write(
        &mut self,
        self_: wasmtime::component::Resource<
            component_bindings::tachyon::mesh::response_body::StreamingResponse,
        >,
        chunk: Vec<u8>,
    ) -> std::result::Result<(), String> {
        let handle =
            wasmtime::component::Resource::<HostStreamingResponseResource>::new_borrow(self_.rep());
        let res = self.table.get(&handle).map_err(|e| format!("{e:#}"))?;
        res.chunk_tx
            .blocking_send(Bytes::from(chunk))
            .map_err(|_| "streaming body channel closed".to_owned())
    }

    fn drop(
        &mut self,
        rep: wasmtime::component::Resource<
            component_bindings::tachyon::mesh::response_body::StreamingResponse,
        >,
    ) -> wasmtime::Result<()> {
        self.table.delete(
            wasmtime::component::Resource::<HostStreamingResponseResource>::new_own(rep.rep()),
        )?;
        Ok(())
    }
}

#[cfg(all(test, feature = "ai-inference"))]
mod stream_budget_tests {
    use super::*;

    #[test]
    fn the_budget_admits_until_it_is_spent_and_again_once_drained() {
        let budget = StreamQueueBudget::default();

        assert!(budget.reserve(STREAM_QUEUE_BUDGET_BYTES - 1));
        // Room for one more byte, so a one-byte event still fits.
        assert!(budget.reserve(1));

        budget.release(STREAM_QUEUE_BUDGET_BYTES);
        // Drained: the next event is admitted immediately rather than waiting.
        assert!(budget.reserve(16));
        budget.release(16);
    }

    #[test]
    fn an_event_larger_than_the_whole_budget_is_still_admitted_when_the_queue_is_empty() {
        // Otherwise a single oversized frame — an upstream may send one up to
        // `MAX_SSE_FRAME_BYTES` — could never be sent at all, and the stream
        // would hang rather than merely be slow.
        let budget = StreamQueueBudget::default();
        assert!(budget.reserve(STREAM_QUEUE_BUDGET_BYTES * 4));
        budget.release(STREAM_QUEUE_BUDGET_BYTES * 4);
    }

    #[test]
    fn a_producer_waiting_for_room_is_released_when_the_consumer_goes() {
        // The failure this prevents: the receiver is dropped while the
        // generation thread is parked on the condvar. A closed channel is not
        // something a sleeping thread can observe, so without the explicit
        // wake it would hold its admission permit until the binding timeout.
        let budget = Arc::new(StreamQueueBudget::default());
        assert!(budget.reserve(STREAM_QUEUE_BUDGET_BYTES));

        let producer = Arc::clone(&budget);
        let waiting = std::thread::spawn(move || producer.reserve(STREAM_QUEUE_BUDGET_BYTES));

        budget.consumer_gone();
        assert!(
            !waiting.join().expect("producer thread should not panic"),
            "a producer woken by a departed consumer must be told to stop, not admitted"
        );
    }

    #[test]
    fn queued_bytes_counts_the_text_that_scales_with_the_answer() {
        assert_eq!(StreamPayload::Content("hello".to_owned()).queued_bytes(), 5);
        assert_eq!(
            StreamPayload::ToolCall(ai_inference::ToolCall {
                id: Some("id".to_owned()),
                name: "read".to_owned(),
                arguments: "{\"p\":1}".to_owned(),
            })
            .queued_bytes(),
            2 + 4 + 7
        );
    }
}
