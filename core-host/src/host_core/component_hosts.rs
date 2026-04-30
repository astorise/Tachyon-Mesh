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
    ) -> std::result::Result<Self, ExecutionError> {
        Ok(Self {
            ctx: build_component_wasi_ctx(route, host_identity.as_ref())?,
            table: ResourceTable::new(),
            limits: GuestResourceLimiter::new(max_memory_bytes),
            secrets,
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
    ) -> std::result::Result<String, String> {
        let loaded = self
            .accelerator_models
            .get(&model_id)
            .ok_or_else(|| format!("accelerator model handle `{model_id}` is unknown"))?;
        if loaded.accelerator != expected_accelerator {
            return Err(format!(
                "accelerator model handle `{model_id}` was loaded for `{}` not `{}`",
                loaded.accelerator.as_str(),
                expected_accelerator.as_str()
            ));
        }
        self.ai_runtime
            .as_ref()
            .ok_or_else(|| "AI inference runtime is unavailable for this component".to_owned())?
            .compute_component_prompt_with_adapter(
                &loaded.alias,
                &prompt,
                self.adapter_id.as_deref(),
            )
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
) -> std::result::Result<WasiCtx, ExecutionError> {
    // Intentionally do not inherit the host environment. Secrets stay in host memory
    // and are only reachable through the typed vault import.
    let mut wasi = WasiCtxBuilder::new();
    for (name, value) in system_runtime_environment(route, host_identity) {
        wasi.env(&name, &value);
    }
    preopen_route_volumes(&mut wasi, route)?;
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

pub(crate) fn preopen_route_volumes(
    wasi: &mut WasiCtxBuilder,
    route: &IntegrityRoute,
) -> std::result::Result<(), ExecutionError> {
    for volume in &route.volumes {
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
    let ciphertext = tde_cipher()
        .encrypt(Nonce::from_slice(&nonce), plaintext)
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
    tde_cipher()
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| anyhow!("failed to decrypt TDE file body"))
}

pub(crate) fn tde_cipher() -> Aes256Gcm {
    Aes256Gcm::new((&tde_key_bytes()).into())
}

pub(crate) fn tde_key_bytes() -> [u8; 32] {
    std::env::var(TDE_KEY_HEX_ENV)
        .ok()
        .and_then(|value| decode_tde_key_hex(value.trim()).ok())
        .unwrap_or([0x42; 32])
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

#[cfg(feature = "ai-inference")]
impl accelerator_component_bindings::tachyon::accelerator::cpu::Host for ComponentHostState {
    fn load_model(&mut self, name: String) -> std::result::Result<u32, String> {
        self.load_accelerator_model(ai_inference::AcceleratorKind::Cpu, name)
    }

    fn compute(&mut self, model_id: u32, prompt: String) -> std::result::Result<String, String> {
        self.compute_accelerator_prompt(ai_inference::AcceleratorKind::Cpu, model_id, prompt)
    }
}

#[cfg(feature = "ai-inference")]
impl accelerator_component_bindings::tachyon::accelerator::gpu::Host for ComponentHostState {
    fn load_model(&mut self, name: String) -> std::result::Result<u32, String> {
        self.load_accelerator_model(ai_inference::AcceleratorKind::Gpu, name)
    }

    fn compute(&mut self, model_id: u32, prompt: String) -> std::result::Result<String, String> {
        self.compute_accelerator_prompt(ai_inference::AcceleratorKind::Gpu, model_id, prompt)
    }
}

#[cfg(feature = "ai-inference")]
impl accelerator_component_bindings::tachyon::accelerator::npu::Host for ComponentHostState {
    fn load_model(&mut self, name: String) -> std::result::Result<u32, String> {
        self.load_accelerator_model(ai_inference::AcceleratorKind::Npu, name)
    }

    fn compute(&mut self, model_id: u32, prompt: String) -> std::result::Result<String, String> {
        self.compute_accelerator_prompt(ai_inference::AcceleratorKind::Npu, model_id, prompt)
    }
}

#[cfg(feature = "ai-inference")]
impl accelerator_component_bindings::tachyon::accelerator::tpu::Host for ComponentHostState {
    fn load_model(&mut self, name: String) -> std::result::Result<u32, String> {
        self.load_accelerator_model(ai_inference::AcceleratorKind::Tpu, name)
    }

    fn compute(&mut self, model_id: u32, prompt: String) -> std::result::Result<String, String> {
        self.compute_accelerator_prompt(ai_inference::AcceleratorKind::Tpu, model_id, prompt)
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
        update_control_plane_route_override(
            self.route_overrides.as_ref(),
            &self.peer_capabilities,
            &route_path,
            &destination,
        )
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
        let url = rewrite_outbound_http_url(&resolved_target.url, &self.runtime_config);

        tracing::info!(
            method = %method,
            url = %url,
            bytes = body.len(),
            "autoscaling guest sending outbound HTTP request"
        );

        let mut request = self.outbound_http_client.request(method, &url);
        for (name, value) in
            filtered_outbound_http_headers(headers, &self.propagated_headers, &resolved_target.kind)
        {
            request = request.header(&name, &value);
        }
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

        Ok(
            background_component_bindings::tachyon::mesh::outbound_http::Response {
                status,
                headers: response_headers,
                body,
            },
        )
    }
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
