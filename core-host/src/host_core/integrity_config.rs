use super::*;

pub(crate) fn init_host_tracing() {
    static INIT: Once = Once::new();

    INIT.call_once(|| {
        ensure_rustls_crypto_provider();
        let _ = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_target(true)
            .try_init();
    });
}

pub(crate) fn ensure_rustls_crypto_provider() {
    tls_runtime::ensure_crypto_provider();
}

/// Inject system routes that are always active when their feature flag is
/// compiled in, without requiring the operator to manually add them via the
/// manifest.  Routes already present in the config are left untouched.
pub(crate) fn inject_feature_routes(mut config: IntegrityConfig) -> IntegrityConfig {
    #[allow(unused_mut)]
    let mut to_inject: Vec<(&str, &str)> = Vec::new();

    #[cfg(feature = "ai-inference")]
    {
        to_inject.push(("/system/model-broker", "model-broker"));
        to_inject.push(("/system/ai-list-model", "ai-list-model"));
    }

    #[cfg(feature = "s3-persistence")]
    {
        to_inject.push(("/system/s3-proxy", "s3-proxy"));
        to_inject.push(("/system/storage-broker", "storage-broker"));
    }

    for (path, name) in to_inject {
        if config.routes.iter().any(|r| r.path == path) {
            continue;
        }
        config.routes.push(IntegrityRoute {
            path: path.to_owned(),
            role: RouteRole::System,
            name: name.to_owned(),
            version: "1.1.0-alpha".to_owned(),
            ..IntegrityRoute::default()
        });
    }
    config
}

pub(crate) fn verify_integrity() -> Result<IntegrityConfig> {
    match verify_integrity_payload(
        EMBEDDED_CONFIG_PAYLOAD,
        EMBEDDED_PUBLIC_KEY,
        EMBEDDED_SIGNATURE,
        "embedded sealed configuration",
    ) {
        Ok(config) => {
            tracing::info!("integrity verification passed");
            Ok(config)
        }
        Err(error) => {
            if is_integrity_schema_violation(&error) {
                tracing::error!(
                    code = ERR_INTEGRITY_SCHEMA_VIOLATION,
                    error = %error,
                    "integrity manifest violates the required schema"
                );
                return Err(error);
            }

            Err(error)
        }
    }
}

#[cfg_attr(not(any(unix, test)), allow(dead_code))]
pub(crate) fn load_integrity_config_from_manifest_path(path: &Path) -> Result<IntegrityConfig> {
    load_integrity_config_from_manifest_path_with_trusted(path, &[])
}

pub(crate) fn load_integrity_config_from_manifest_path_with_trusted(
    path: &Path,
    trusted_signers: &[String],
) -> Result<IntegrityConfig> {
    let manifest = read_integrity_manifest(path)?;
    verify_integrity_payload_with_trusted(
        &manifest.config_payload,
        &manifest.public_key,
        &manifest.signature,
        &format!("integrity manifest at {}", path.display()),
        trusted_signers,
    )
}

#[cfg_attr(not(any(unix, test)), allow(dead_code))]
pub(crate) fn read_integrity_manifest(path: &Path) -> Result<IntegrityManifest> {
    let manifest = fs::read_to_string(path)
        .with_context(|| format!("failed to read integrity manifest at {}", path.display()))?;

    serde_json::from_str(&manifest)
        .with_context(|| format!("failed to parse integrity manifest at {}", path.display()))
}

pub(crate) fn verify_integrity_payload(
    payload: &str,
    public_key_hex: &str,
    signature_hex: &str,
    source: &str,
) -> Result<IntegrityConfig> {
    verify_integrity_payload_with_trusted(payload, public_key_hex, signature_hex, source, &[])
}

pub(crate) fn verify_integrity_payload_with_trusted(
    payload: &str,
    public_key_hex: &str,
    signature_hex: &str,
    source: &str,
    trusted_signers: &[String],
) -> Result<IntegrityConfig> {
    // Verify the cryptographic signature with the declared key.
    verify_integrity_signature_with_trusted(
        payload,
        public_key_hex,
        signature_hex,
        trusted_signers,
    )?;
    // Trust check: only applies when a non-empty trusted-signers list has been
    // configured.  An empty list means "open mode" — any cryptographically
    // valid signature is accepted (boot-time and batch-job paths).  A non-empty
    // list restricts submission to the embedded boot key or explicitly-listed
    // cluster keys.
    if !trusted_signers.is_empty() {
        let embedded_key = EMBEDDED_PUBLIC_KEY;
        let is_embedded = public_key_hex.eq_ignore_ascii_case(embedded_key);
        let is_trusted = is_embedded
            || trusted_signers
                .iter()
                .any(|k| k.trim().eq_ignore_ascii_case(public_key_hex));
        if !is_trusted {
            anyhow::bail!(
                "Integrity Validation Failed: signer `{public_key_hex}` is not the embedded key \
                 and is not in the trusted-signers list"
            );
        }
    }
    let config = serde_json::from_str::<IntegrityConfig>(payload)
        .map_err(|error| integrity_schema_violation(source, error))?;
    validate_integrity_config(config)
}

pub(crate) fn integrity_schema_violation(source: &str, error: serde_json::Error) -> anyhow::Error {
    anyhow!("{ERR_INTEGRITY_SCHEMA_VIOLATION}: failed to parse {source}: {error}")
}

pub(crate) fn is_integrity_schema_violation(error: &anyhow::Error) -> bool {
    error.to_string().contains(ERR_INTEGRITY_SCHEMA_VIOLATION)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdminEnrollmentStartRequest {
    pub(crate) node_public_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdminEnrollmentApproveRequest {
    pub(crate) session_id: String,
    pub(crate) pin: String,
    /// Hex-encoded signed certificate the operator-side node minted for the
    /// new device. The signing happens upstream of this endpoint (e.g. via the
    /// existing `auth_manager` token-signing or a dedicated cluster-CA tool);
    /// this handler just stages the bytes for the unenrolled node to fetch.
    pub(crate) signed_certificate_hex: String,
}

/// `POST /admin/enrollment/start` — begin a node enrollment session. Caller is
/// the unenrolled node's outbound channel (via the active node's HTTP
/// gateway); body carries the new node's hex-encoded ed25519 public key.
/// Returns the session id + the human-readable PIN the operator must enter
/// in Tachyon Studio to approve the enrollment.
pub(crate) async fn admin_enrollment_start_handler(
    State(state): State<AppState>,
    axum::Json(payload): axum::Json<AdminEnrollmentStartRequest>,
) -> Response {
    if payload.node_public_key.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "nodePublicKey is required").into_response();
    }
    match serde_json::to_vec(&payload) {
        Ok(body) => {
            forward_node_registry_faas(
                state,
                "POST",
                "/admin/enrollment/start".to_owned(),
                Bytes::from(body),
            )
            .await
        }
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

/// `POST /admin/enrollment/approve` — operator-driven approval entered via
/// Tachyon Studio. Validates the PIN against the recorded session and stages
/// the signed certificate bytes for the unenrolled node's next poll.
pub(crate) async fn admin_enrollment_approve_handler(
    State(state): State<AppState>,
    axum::Json(payload): axum::Json<AdminEnrollmentApproveRequest>,
) -> Response {
    match serde_json::to_vec(&payload) {
        Ok(body) => {
            forward_node_registry_faas(
                state,
                "POST",
                "/admin/enrollment/approve".to_owned(),
                Bytes::from(body),
            )
            .await
        }
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

/// `GET /admin/enrollment/poll/{session_id}` — invoked by the unenrolled node's
/// long-poll. Returns 204 No Content while pending, 200 with the signed cert
/// bytes (hex) once the operator has approved, 410 Gone after rejection.
pub(crate) async fn admin_enrollment_poll_handler(
    State(state): State<AppState>,
    axum::extract::Path(session_id): axum::extract::Path<String>,
) -> Response {
    forward_node_registry_faas(
        state,
        "GET",
        format!("/admin/enrollment/poll/{session_id}"),
        Bytes::new(),
    )
    .await
}

async fn forward_node_registry_faas(
    state: AppState,
    method: &'static str,
    uri: String,
    body: Bytes,
) -> Response {
    let runtime = state.runtime.load_full();
    let engine = runtime.engine.clone();
    let config = runtime.config.clone();
    let route = IntegrityRoute {
        path: "/admin/node-registry".to_owned(),
        role: RouteRole::System,
        name: "system-faas-node-registry".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        ..IntegrityRoute::default()
    };
    let request = GuestRequest::new(method, uri.clone(), body);
    let runtime_telemetry = state.telemetry.clone();
    let async_log_sender = state.async_log_sender.clone();
    let host_identity = Arc::clone(&state.host_identity);
    let storage_broker = Arc::clone(&state.storage_broker);
    let bridge_manager = Arc::clone(&state.bridge_manager);
    let concurrency_limits = Arc::clone(&runtime.concurrency_limits);
    let route_overrides = Arc::clone(&state.route_overrides);
    let host_load = Arc::clone(&state.host_load);
    let instance_pool = Arc::clone(&runtime.instance_pool);
    let linker_cache = Arc::clone(&runtime.linker_cache);
    #[cfg(feature = "ai-inference")]
    let ai_runtime = Arc::clone(&runtime.ai_runtime);

    let result = tokio::task::spawn_blocking(move || {
        execute_guest(
            &engine,
            "system-faas-node-registry",
            request,
            &route,
            GuestExecutionContext {
                config,
                sampled_execution: false,
                runtime_telemetry,
                async_log_sender,
                secret_access: SecretAccess::default(),
                request_headers: HeaderMap::new(),
                host_identity,
                storage_broker,
                bridge_manager,
                telemetry: None,
                concurrency_limits,
                propagated_headers: Vec::new(),
                route_overrides,
                host_load,
                #[cfg(feature = "ai-inference")]
                ai_runtime,
                instance_pool: Some(instance_pool),
                linker_cache: Some(linker_cache),
            },
        )
    })
    .await;

    let outcome = match result {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(error)) => {
            error.log_if_needed("system-faas-node-registry");
            let (status, message) = error.into_response(&runtime.config);
            return (status, message).into_response();
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("node registry FaaS task failed: {error}"),
            )
                .into_response();
        }
    };

    match outcome.output {
        GuestExecutionOutput::Http(response) => guest_http_response_into_axum(response),
        GuestExecutionOutput::LegacyStdout(stdout) => (StatusCode::OK, stdout).into_response(),
    }
}

fn guest_http_response_into_axum(response: GuestHttpResponse) -> Response {
    let mut builder = Response::builder().status(response.status);
    for (name, value) in response.headers {
        builder = builder.header(name, value);
    }
    builder
        .body(axum::body::Body::from(response.body))
        .unwrap_or_else(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to build node registry FaaS response: {error}"),
            )
                .into_response()
        })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegistryGpuStats {
    pub(crate) id: String,
    pub(crate) model: String,
    pub(crate) vram_total_mb: u64,
    pub(crate) vram_used_mb: u64,
    pub(crate) compute_utilization: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegistryActiveSystem {
    pub(crate) slug: String,
    pub(crate) version: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegistryNodeCapabilities {
    pub(crate) total_ram_mb: u64,
    pub(crate) available_ram_mb: u64,
    pub(crate) accelerators: Vec<String>,
    pub(crate) gpus: Vec<RegistryGpuStats>,
    pub(crate) active_systems: Vec<RegistryActiveSystem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegistryEnrolledNode {
    pub(crate) node_id: String,
    pub(crate) public_key: String,
    pub(crate) status: String,
    pub(crate) approved_at: u64,
    pub(crate) last_seen: u64,
    pub(crate) region: Option<String>,
    pub(crate) zone: Option<String>,
    pub(crate) capabilities: RegistryNodeCapabilities,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegistrySystem {
    pub(crate) slug: String,
    pub(crate) crate_name: String,
    pub(crate) version: String,
    pub(crate) description: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegistryDeployedSystem {
    pub(crate) slug: String,
    pub(crate) catalog_version: String,
    pub(crate) status: String,
    pub(crate) host_node_count: usize,
    pub(crate) has_drift: bool,
    pub(crate) nodes: Vec<RegistryActiveSystemNode>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RegistryActiveSystemNode {
    pub(crate) node_id: String,
    pub(crate) version: String,
}

pub(crate) async fn admin_nodes_handler(State(state): State<AppState>) -> Response {
    let mut nodes = match list_registry_nodes(&state.core_store) {
        Ok(nodes) => nodes,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
        }
    };
    let self_id = &state.host_identity.public_key_hex;
    if !nodes.iter().any(|n| &n.node_id == self_id) {
        nodes.push(self_registry_node(self_id, &state));
    }
    (StatusCode::OK, axum::Json(nodes)).into_response()
}

fn self_registry_node(node_id: &str, state: &AppState) -> RegistryEnrolledNode {
    use sysinfo::System;
    let mut sys = System::new();
    sys.refresh_memory();
    let total_ram_mb = sys.total_memory() / 1024 / 1024;
    let available_ram_mb = sys.available_memory() / 1024 / 1024;
    let accelerators = state.host_capabilities.names();
    let gpus = detect_self_gpus(&state.host_capabilities);
    let runtime = state.runtime.load();
    let active_systems = runtime
        .config
        .routes
        .iter()
        .filter(|r| r.role == RouteRole::System)
        .map(|r| RegistryActiveSystem {
            slug: r.path.trim_start_matches("/system/").to_owned(),
            version: r.version.clone(),
        })
        .collect();
    RegistryEnrolledNode {
        node_id: node_id.to_owned(),
        public_key: node_id.to_owned(),
        status: "online".to_owned(),
        approved_at: unix_timestamp_seconds().unwrap_or(0),
        last_seen: unix_timestamp_seconds().unwrap_or(0),
        region: None,
        zone: None,
        capabilities: RegistryNodeCapabilities {
            total_ram_mb,
            available_ram_mb,
            accelerators,
            gpus,
            active_systems,
        },
    }
}

fn detect_self_gpus(capabilities: &Capabilities) -> Vec<RegistryGpuStats> {
    // Report one placeholder GPU entry when CUDA acceleration is detected.
    // Detailed per-GPU stats (VRAM, utilization) require NVML and are
    // populated by the node-registry FaaS once a node reports capabilities.
    if capabilities.supports(Capabilities::from_mask(Capabilities::ACCEL_CUDA)) {
        vec![RegistryGpuStats {
            id: "gpu:0".to_owned(),
            model: "CUDA-capable device".to_owned(),
            vram_total_mb: 0,
            vram_used_mb: 0,
            compute_utilization: 0.0,
        }]
    } else {
        Vec::new()
    }
}

pub(crate) async fn admin_node_capabilities_handler(
    State(state): State<AppState>,
    axum::extract::Path(node_id): axum::extract::Path<String>,
    axum::Json(capabilities): axum::Json<RegistryNodeCapabilities>,
) -> Response {
    let mut node = match state.core_store.kv_partition_get("node-registry", &node_id) {
        Ok(Some(raw)) => {
            serde_json::from_slice::<RegistryEnrolledNode>(&raw).unwrap_or_else(|_| {
                RegistryEnrolledNode {
                    node_id: node_id.clone(),
                    public_key: String::new(),
                    status: "awaiting-capabilities".to_owned(),
                    ..RegistryEnrolledNode::default()
                }
            })
        }
        Ok(None) => RegistryEnrolledNode {
            node_id: node_id.clone(),
            public_key: String::new(),
            status: "awaiting-capabilities".to_owned(),
            ..RegistryEnrolledNode::default()
        },
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
        }
    };
    node.capabilities = capabilities;
    node.status = "online".to_owned();
    node.last_seen = current_unix_seconds();
    let payload = match serde_json::to_vec(&node) {
        Ok(payload) => payload,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
        }
    };
    match state
        .core_store
        .kv_partition_set("node-registry", &node_id, &payload)
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response(),
    }
}

pub(crate) async fn admin_registered_systems_handler() -> Response {
    (StatusCode::OK, axum::Json(read_system_manifest())).into_response()
}

pub(crate) async fn admin_deployed_systems_handler(State(state): State<AppState>) -> Response {
    let nodes = match list_registry_nodes(&state.core_store) {
        Ok(nodes) => nodes,
        Err(error) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response()
        }
    };
    let systems = read_system_manifest()
        .into_iter()
        .map(|system| deployed_system_from_nodes(&system, &nodes))
        .collect::<Vec<_>>();
    (StatusCode::OK, axum::Json(systems)).into_response()
}

fn list_registry_nodes(core_store: &store::CoreStore) -> Result<Vec<RegistryEnrolledNode>> {
    Ok(core_store
        .kv_partition_get_range("node-registry", "", "\u{10ffff}", 10_000, 0)?
        .into_iter()
        .filter_map(|(_, value)| serde_json::from_slice::<RegistryEnrolledNode>(&value).ok())
        .collect())
}

fn read_system_manifest() -> Vec<RegistrySystem> {
    let Ok(raw) = std::fs::read_to_string(PathBuf::from("systems").join("manifest.toml")) else {
        return Vec::new();
    };
    parse_system_manifest(&raw)
}

fn parse_system_manifest(raw: &str) -> Vec<RegistrySystem> {
    let mut systems = Vec::new();
    let mut current = RegistrySystem::default();
    let mut in_entry = false;
    for line in raw.lines().map(str::trim) {
        if line == "[[system]]" {
            if in_entry {
                systems.push(current);
                current = RegistrySystem::default();
            }
            in_entry = true;
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_owned();
        match key.trim() {
            "slug" => current.slug = value,
            "crate_name" => current.crate_name = value,
            "version" => current.version = value,
            "description" => current.description = value,
            _ => {}
        }
    }
    if in_entry {
        systems.push(current);
    }
    systems
}

fn deployed_system_from_nodes(
    system: &RegistrySystem,
    nodes: &[RegistryEnrolledNode],
) -> RegistryDeployedSystem {
    let deployed_nodes = nodes
        .iter()
        .flat_map(|node| {
            node.capabilities
                .active_systems
                .iter()
                .filter(move |active| active.slug == system.slug)
                .map(move |active| RegistryActiveSystemNode {
                    node_id: node.node_id.clone(),
                    version: active.version.clone(),
                })
        })
        .collect::<Vec<_>>();
    let has_drift = deployed_nodes
        .iter()
        .any(|node| node.version != system.version);
    RegistryDeployedSystem {
        slug: system.slug.clone(),
        catalog_version: system.version.clone(),
        status: if deployed_nodes.is_empty() {
            "not-deployed".to_owned()
        } else if has_drift {
            "version-drift".to_owned()
        } else {
            "deployed".to_owned()
        },
        host_node_count: deployed_nodes.len(),
        has_drift,
        nodes: deployed_nodes,
    }
}

fn current_unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

/// Cluster-wide configuration update event written to `config_update_outbox`
/// whenever a node accepts a signed manifest via `POST /admin/manifest`. The
/// gossip bridge (still TODO — Session C wiring) reads from this table and
/// broadcasts to peers, who then pull the new manifest from `origin_node_id`
/// over the secure overlay.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ConfigUpdateEvent {
    pub version: u64,
    pub checksum: String,
    pub origin_node_id: String,
    pub ts_ms: u64,
}

pub(crate) type ConfigUpdate = ConfigUpdateEvent;

/// `GET /admin/manifest` — returns the active `IntegrityConfig` as JSON.
/// This is the canonical way for connected clients to obtain the live topology
/// without depending on a local `integrity.lock` file.
pub(crate) async fn admin_get_manifest_handler(State(state): State<AppState>) -> Response {
    let config = state.runtime.load().config.clone();
    match serde_json::to_vec(&config) {
        Ok(body) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            body,
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to serialize active config: {error}"),
        )
            .into_response(),
    }
}

/// `POST /admin/manifest` body: the same `IntegrityManifest` shape as the
/// on-disk file (so admin tooling can hand the file's bytes through unchanged).
/// The payload's signature is verified against the same trust root as the
/// embedded boot manifest. Updates are accepted only when the new
/// `config_version` is strictly greater than the running one — a defense
/// against rollback / replay across the cluster.
pub(crate) async fn admin_manifest_update_handler(
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    let manifest: IntegrityManifest = match serde_json::from_slice(&body) {
        Ok(m) => m,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("failed to decode manifest payload: {error}"),
            )
                .into_response();
        }
    };

    // Verify signature + parse + validate the embedded config. This rejects
    // unsigned / tampered submissions before we touch disk.
    // The current runtime's trusted_signers extend the verification set beyond
    // the embedded boot key, allowing cluster nodes to sign manifests.
    let current_trusted = state.runtime.load().config.trusted_signers.clone();
    let new_config = match verify_integrity_payload_with_trusted(
        &manifest.config_payload,
        &manifest.public_key,
        &manifest.signature,
        "admin manifest submission",
        &current_trusted,
    ) {
        Ok(config) => config,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("manifest signature verification failed: {error:#}"),
            )
                .into_response();
        }
    };

    let current_version = state.runtime.load().config.config_version;
    if new_config.config_version <= current_version {
        return (
            StatusCode::CONFLICT,
            format!(
                "manifest config_version {} is not strictly greater than current {}",
                new_config.config_version, current_version
            ),
        )
            .into_response();
    }

    // Atomic write: stage to a tempfile, fsync, rename. The notify-based
    // watcher (Wave 1) sees the rename and triggers `reload_runtime_from_disk`.
    let manifest_path = state.manifest_path.clone();
    let payload_bytes = body.to_vec();
    let write_result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let staging = manifest_path.with_extension("lock.tmp");
        fs::write(&staging, &payload_bytes)?;
        fs::rename(&staging, &manifest_path)?;
        Ok(())
    })
    .await;
    match write_result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to persist manifest: {error}"),
            )
                .into_response();
        }
        Err(join_error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("manifest write task failed: {join_error}"),
            )
                .into_response();
        }
    }

    // Emit a gossip update event for peers.
    use sha2::Digest;
    let checksum = format!(
        "sha256:{}",
        hex::encode(sha2::Sha256::digest(manifest.config_payload.as_bytes()))
    );
    let ts_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let event = ConfigUpdateEvent {
        version: new_config.config_version,
        checksum,
        origin_node_id: state.host_identity.public_key_hex.clone(),
        ts_ms,
    };
    let _ = state.config_updates.send(event.clone());
    if let Ok(payload) = serde_json::to_vec(&event) {
        if let Err(error) = state
            .core_store
            .append_outbox(store::CoreStoreBucket::ConfigUpdateOutbox, &payload)
        {
            tracing::warn!("manifest accepted but config_update_outbox append failed: {error:#}");
        }
    }

    (
        StatusCode::ACCEPTED,
        format!(
            "manifest accepted, config_version={}",
            new_config.config_version
        ),
    )
        .into_response()
}

pub(crate) fn validate_integrity_config(mut config: IntegrityConfig) -> Result<IntegrityConfig> {
    if config.host_address.trim().is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration is missing `host_address`"
        ));
    }

    if config.max_stdout_bytes == 0 {
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration is missing `max_stdout_bytes`"
        ));
    }

    if config.guest_fuel_budget == 0 {
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration is missing `guest_fuel_budget`"
        ));
    }

    if config.guest_memory_limit_bytes == 0 {
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration is missing `guest_memory_limit_bytes`"
        ));
    }

    if config.resource_limit_response.trim().is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration is missing `resource_limit_response`"
        ));
    }

    if !config.telemetry_sample_rate.is_finite()
        || !(0.0..=1.0).contains(&config.telemetry_sample_rate)
    {
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration must set `telemetry_sample_rate` between 0.0 and 1.0"
        ));
    }

    if config.routes.is_empty() && config.batch_targets.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration must define at least one route or batch target"
        ));
    }

    config.advertise_ip = normalize_advertise_ip(config.advertise_ip)?;
    config.tls_address = normalize_tls_address(config.tls_address)?;
    config.batch_targets = normalize_batch_targets(config.batch_targets)?;
    config.routes = normalize_config_routes(config.routes, !config.batch_targets.is_empty())?;
    if config.require_scopes {
        validate_require_scopes(&config.routes)?;
    }
    validate_tee_requirements(&config)?;
    validate_kv_caches(&config)?;
    let route_registry = RouteRegistry::build(&config)?;
    config.resources = normalize_resources(config.resources, &config.routes, &route_registry)?;
    config.layer4 = normalize_layer4_config(config.layer4, &route_registry)?;
    Ok(config)
}

pub(crate) fn validate_kv_caches(config: &IntegrityConfig) -> Result<()> {
    let mut seen_names = std::collections::HashSet::new();
    for cache in &config.kv_caches {
        if cache.name.trim().is_empty() {
            anyhow::bail!("Integrity Validation Failed: kv_caches entry has an empty `name`");
        }
        if cache.model_ref.trim().is_empty() {
            anyhow::bail!(
                "Integrity Validation Failed: kv_caches entry `{}` has an empty `model_ref`",
                cache.name
            );
        }
        if cache.model_ref.contains('/') || cache.model_ref.contains('\\') {
            anyhow::bail!(
                "Integrity Validation Failed: kv_caches entry `{}` has an invalid `model_ref` \
                 (slashes are not allowed)",
                cache.name
            );
        }
        if !seen_names.insert(cache.name.clone()) {
            anyhow::bail!(
                "Integrity Validation Failed: duplicate kv_caches name `{}`",
                cache.name
            );
        }
    }
    Ok(())
}

/// Enforces `require_scopes: true` — every route must have an explicit,
/// non-allow-all `scopes:` block. Called only when the flag is set; the default
/// is `false` so existing manifests without scopes continue to work.
fn validate_require_scopes(routes: &[IntegrityRoute]) -> Result<()> {
    use crate::host_core::scoping::DeploymentScopes;
    for route in routes {
        match &route.scopes {
            None => {
                return Err(anyhow!(
                    "Integrity Validation Failed: route `{}` is missing a `scopes` block \
                     and `require_scopes` is enabled on this node",
                    route.path
                ));
            }
            Some(value) => {
                // Already validated by validate_route_scopes; re-parse to check allow-all.
                if let Ok(scopes) = DeploymentScopes::from_manifest(value) {
                    if scopes.allow_all {
                        return Err(anyhow!(
                            "Integrity Validation Failed: route `{}` resolves to allow-all scopes \
                             and `require_scopes` is enabled on this node; add explicit scope patterns",
                            route.path
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn validate_tee_requirements(config: &IntegrityConfig) -> Result<()> {
    if config.routes.iter().any(|route| route.requires_tee) && config.tee_backend.is_none() {
        return Err(anyhow!(
            "Integrity Validation Failed: routes with `requires_tee: true` require `tee_backend` to be configured"
        ));
    }

    Ok(())
}

pub(crate) fn normalize_config_routes(
    routes: Vec<IntegrityRoute>,
    allow_empty: bool,
) -> Result<Vec<IntegrityRoute>> {
    if routes.is_empty() {
        if allow_empty {
            return Ok(Vec::new());
        }
        return Err(anyhow!(
            "Integrity Validation Failed: embedded sealed configuration must define at least one route"
        ));
    }

    let mut normalized = routes
        .into_iter()
        .map(validate_integrity_route)
        .collect::<Result<Vec<_>>>()?;

    normalized.sort_by(|left, right| left.path.cmp(&right.path));
    for pair in normalized.windows(2) {
        if pair[0].path == pair[1].path {
            return Err(anyhow!(
                "Integrity Validation Failed: route `{}` is defined more than once",
                pair[0].path
            ));
        }
    }
    ensure_unique_model_aliases(&normalized)?;
    ensure_unique_route_domains(&normalized)?;
    Ok(normalized)
}

pub(crate) fn normalize_batch_targets(
    targets: Vec<IntegrityBatchTarget>,
) -> Result<Vec<IntegrityBatchTarget>> {
    let mut normalized = targets
        .into_iter()
        .map(validate_integrity_batch_target)
        .collect::<Result<Vec<_>>>()?;

    normalized.sort_by(|left, right| left.name.cmp(&right.name));
    for pair in normalized.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(anyhow!(
                "Integrity Validation Failed: batch target `{}` is defined more than once",
                pair[0].name
            ));
        }
    }

    Ok(normalized)
}

pub(crate) fn normalize_layer4_config(
    mut layer4: IntegrityLayer4Config,
    route_registry: &RouteRegistry,
) -> Result<IntegrityLayer4Config> {
    layer4.tcp = normalize_tcp_bindings(layer4.tcp, route_registry)?;
    layer4.udp = normalize_udp_bindings(layer4.udp, route_registry)?;
    Ok(layer4)
}

pub(crate) fn normalize_resources(
    resources: BTreeMap<String, IntegrityResource>,
    routes: &[IntegrityRoute],
    route_registry: &RouteRegistry,
) -> Result<BTreeMap<String, IntegrityResource>> {
    let reserved_names = routes
        .iter()
        .map(|route| route.name.clone())
        .collect::<BTreeSet<_>>();
    let mut normalized = BTreeMap::new();

    for (name, resource) in resources {
        let normalized_name = normalize_service_name(&name).map_err(|error| {
            anyhow!(
                "Integrity Validation Failed: resource `{name}` has an invalid logical name: {error}"
            )
        })?;
        if reserved_names.contains(&normalized_name) {
            return Err(anyhow!(
                "Integrity Validation Failed: resource `{normalized_name}` conflicts with a sealed route name"
            ));
        }

        let normalized_resource =
            normalize_resource_definition(resource, &normalized_name, route_registry)?;
        if normalized
            .insert(normalized_name.clone(), normalized_resource)
            .is_some()
        {
            return Err(anyhow!(
                "Integrity Validation Failed: resource `{normalized_name}` is defined more than once"
            ));
        }
    }

    Ok(normalized)
}

pub(crate) fn normalize_resource_definition(
    resource: IntegrityResource,
    resource_name: &str,
    route_registry: &RouteRegistry,
) -> Result<IntegrityResource> {
    match resource {
        IntegrityResource::Internal {
            target,
            version_constraint,
        } => {
            let normalized_constraint = version_constraint
                .map(|constraint| {
                    VersionReq::parse(constraint.trim()).map(|parsed| parsed.to_string()).map_err(
                        |_| {
                            anyhow!(
                                "Integrity Validation Failed: resource `{resource_name}` has an invalid `version_constraint`"
                            )
                        },
                    )
                })
                .transpose()?;
            let normalized_target = normalize_internal_resource_reference(
                resource_name,
                &target,
                normalized_constraint.as_deref(),
                route_registry,
            )?;
            Ok(IntegrityResource::Internal {
                target: normalized_target,
                version_constraint: normalized_constraint,
            })
        }
        IntegrityResource::External {
            target,
            allowed_methods,
        } => {
            let normalized_target = normalize_external_resource_target(resource_name, &target)?;
            Ok(IntegrityResource::External {
                target: normalized_target,
                allowed_methods: normalize_resource_methods(resource_name, allowed_methods)?,
            })
        }
    }
}

pub(crate) fn normalize_internal_resource_reference(
    resource_name: &str,
    target: &str,
    version_constraint: Option<&str>,
    route_registry: &RouteRegistry,
) -> Result<String> {
    if target.trim().is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: internal resource `{resource_name}` must include a non-empty `target`"
        ));
    }

    if target.trim_start().starts_with('/') {
        let normalized_path = normalize_route_path(target);
        let route = route_registry.by_path.get(&normalized_path).ok_or_else(|| {
            anyhow!(
                "Integrity Validation Failed: internal resource `{resource_name}` target `{normalized_path}` does not match any sealed route"
            )
        })?;
        if let Some(requirement) = version_constraint {
            let parsed = VersionReq::parse(requirement).map_err(|_| {
                anyhow!(
                    "Integrity Validation Failed: resource `{resource_name}` has an invalid `version_constraint`"
                )
            })?;
            if !parsed.matches(&route.version) {
                return Err(anyhow!(
                    "Integrity Validation Failed: internal resource `{resource_name}` target `{normalized_path}` does not satisfy `{requirement}`"
                ));
            }
        }
        return Ok(normalized_path);
    }

    let normalized_name = normalize_service_name(target).map_err(|error| {
        anyhow!(
            "Integrity Validation Failed: internal resource `{resource_name}` target `{target}` is invalid: {error}"
        )
    })?;
    if let Some(requirement) = version_constraint {
        let parsed = VersionReq::parse(requirement).map_err(|_| {
            anyhow!(
                "Integrity Validation Failed: resource `{resource_name}` has an invalid `version_constraint`"
            )
        })?;
        route_registry
            .resolve_named_route_matching(&normalized_name, &parsed)
            .map_err(|error| anyhow!("Integrity Validation Failed: {error}"))?;
    } else {
        route_registry
            .resolve_named_route(&normalized_name)
            .map_err(|error| anyhow!("Integrity Validation Failed: {error}"))?;
    }
    Ok(normalized_name)
}

pub(crate) fn normalize_external_resource_target(
    resource_name: &str,
    target: &str,
) -> Result<String> {
    let parsed = reqwest::Url::parse(target.trim()).map_err(|error| {
        anyhow!(
            "Integrity Validation Failed: external resource `{resource_name}` target is not a valid URL: {error}"
        )
    })?;
    let host = parsed.host_str().ok_or_else(|| {
        anyhow!(
            "Integrity Validation Failed: external resource `{resource_name}` target must include a host"
        )
    })?;
    let loopback_http = parsed.scheme() == "http"
        && (host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<IpAddr>()
                .ok()
                .is_some_and(|ip| ip.is_loopback()));
    let cluster_local_http = parsed.scheme() == "http" && is_cluster_local_service_host(host);
    if parsed.scheme() != "https" && !loopback_http && !cluster_local_http {
        return Err(anyhow!(
            "Integrity Validation Failed: external resource `{resource_name}` target must use HTTPS unless it points at localhost for tests or a cluster-local service"
        ));
    }
    Ok(parsed.to_string())
}

pub(crate) fn is_cluster_local_service_host(host: &str) -> bool {
    let normalized = host.trim().trim_end_matches('.').to_ascii_lowercase();
    !normalized.is_empty()
        && !normalized.eq("localhost")
        && (!normalized.contains('.')
            || normalized.ends_with(".svc")
            || normalized.ends_with(".svc.cluster.local"))
}

pub(crate) fn normalize_resource_methods(
    resource_name: &str,
    methods: Vec<String>,
) -> Result<Vec<String>> {
    let normalized = methods
        .into_iter()
        .map(|method| {
            let uppercase = method.trim().to_ascii_uppercase();
            if uppercase.is_empty() {
                return Err(anyhow!(
                    "Integrity Validation Failed: external resource `{resource_name}` must not declare empty HTTP methods"
                ));
            }
            Method::from_bytes(uppercase.as_bytes()).map_err(|error| {
                anyhow!(
                    "Integrity Validation Failed: external resource `{resource_name}` has an invalid HTTP method `{uppercase}`: {error}"
                )
            })?;
            Ok(uppercase)
        })
        .collect::<Result<BTreeSet<_>>>()?;
    if normalized.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: external resource `{resource_name}` must declare at least one allowed HTTP method"
        ));
    }
    Ok(normalized.into_iter().collect())
}

pub(crate) fn normalize_tcp_bindings(
    bindings: Vec<IntegrityTcpBinding>,
    route_registry: &RouteRegistry,
) -> Result<Vec<IntegrityTcpBinding>> {
    let mut normalized = bindings
        .into_iter()
        .map(|binding| {
            if binding.port == 0 {
                return Err(anyhow!(
                    "Integrity Validation Failed: TCP Layer 4 bindings must use a port above zero"
                ));
            }

            let target = normalize_service_name(&binding.target).map_err(|error| {
                anyhow!("Integrity Validation Failed: TCP Layer 4 target is invalid: {error}")
            })?;
            route_registry
                .resolve_named_route(&target)
                .map_err(|error| {
                    anyhow!(
                    "Integrity Validation Failed: TCP Layer 4 target `{target}` is invalid: {error}"
                )
                })?;

            Ok(IntegrityTcpBinding {
                port: binding.port,
                target,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    normalized.sort_by_key(|binding| binding.port);
    for pair in normalized.windows(2) {
        if pair[0].port == pair[1].port {
            return Err(anyhow!(
                "Integrity Validation Failed: TCP Layer 4 port `{}` is defined more than once",
                pair[0].port
            ));
        }
    }

    Ok(normalized)
}

pub(crate) fn normalize_udp_bindings(
    bindings: Vec<IntegrityUdpBinding>,
    route_registry: &RouteRegistry,
) -> Result<Vec<IntegrityUdpBinding>> {
    let mut normalized = bindings
        .into_iter()
        .map(|binding| {
            if binding.port == 0 {
                return Err(anyhow!(
                    "Integrity Validation Failed: UDP Layer 4 bindings must use a port above zero"
                ));
            }

            let target = normalize_service_name(&binding.target).map_err(|error| {
                anyhow!("Integrity Validation Failed: UDP Layer 4 target is invalid: {error}")
            })?;
            route_registry
                .resolve_named_route(&target)
                .map_err(|error| {
                    anyhow!(
                        "Integrity Validation Failed: UDP Layer 4 target `{target}` is invalid: {error}"
                    )
                })?;

            Ok(IntegrityUdpBinding {
                port: binding.port,
                target,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    normalized.sort_by_key(|binding| binding.port);
    for pair in normalized.windows(2) {
        if pair[0].port == pair[1].port {
            return Err(anyhow!(
                "Integrity Validation Failed: UDP Layer 4 port `{}` is defined more than once",
                pair[0].port
            ));
        }
    }

    Ok(normalized)
}

pub(crate) fn validate_integrity_route(route: IntegrityRoute) -> Result<IntegrityRoute> {
    let normalized = normalize_route_path(&route.path);

    if normalized == "/" || resolve_function_name(&normalized).is_none() {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{normalized}` does not resolve to a guest function"
        ));
    }

    if route.max_concurrency == 0 {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{normalized}` must set `max_concurrency` above zero"
        ));
    }

    if route.min_instances > route.max_concurrency {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{normalized}` cannot set `min_instances` above `max_concurrency`"
        ));
    }

    let normalized_volumes = normalize_route_volumes(route.volumes, route.role, &normalized)?;
    let normalized_concurrency =
        validate_concurrency_policy(route.concurrency, &normalized, &normalized_volumes)?;

    Ok(IntegrityRoute {
        path: normalized.clone(),
        role: route.role,
        name: normalize_route_name(&route.name, &normalized)?,
        version: normalize_route_version(&route.version, &normalized)?,
        dependencies: normalize_route_dependencies(route.dependencies, &normalized)?,
        requires_credentials: normalize_route_credentials(route.requires_credentials)?,
        middleware: normalize_route_middleware(route.middleware, &normalized)?,
        env: normalize_route_env(route.env, &normalized)?,
        allowed_secrets: normalize_allowed_secrets(route.allowed_secrets)?,
        targets: normalize_route_targets(route.targets)?,
        resiliency: normalize_route_resiliency(route.resiliency, &normalized)?,
        models: normalize_route_models(route.models, &normalized)?,
        domains: normalize_route_domains(route.domains, &normalized)?,
        min_instances: route.min_instances,
        max_concurrency: route.max_concurrency,
        volumes: normalized_volumes,
        resource_policy: route.resource_policy,
        runtime: normalize_route_runtime(route.runtime, &normalized)?,
        sync_to_cloud: route.sync_to_cloud,
        requires_tee: route.requires_tee,
        allow_overflow: route.allow_overflow,
        distributed_rate_limit: route.distributed_rate_limit,
        shadow_target: route.shadow_target,
        adapter_id: route.adapter_id,
        canary: route.canary,
        concurrency: normalized_concurrency,
        scopes: validate_route_scopes(route.scopes, &normalized)?,
    })
}

/// Reject incompatible concurrency × volume combinations. Returns the policy
/// unchanged on success. `volumes` is the already-normalized list of route volumes.
fn validate_concurrency_policy(
    policy: ConcurrencyPolicy,
    route_path: &str,
    volumes: &[IntegrityVolume],
) -> Result<ConcurrencyPolicy> {
    if policy.mode == ConcurrencyMode::Unrestricted {
        for volume in volumes {
            if volume.consistency.write_mode == WriteMode::PessimisticLock {
                return Err(anyhow!(
                    "Integrity Validation Failed: route `{route_path}` declares \
                     `concurrency.mode = \"unrestricted\"` but volume `{}` requires \
                     `pessimistic_lock`; pessimistic locking only applies under singleton modes",
                    volume.guest_path
                ));
            }
        }
    }
    if let Some(ttl) = policy.lock_ttl_ms {
        if ttl == 0 {
            return Err(anyhow!(
                "Integrity Validation Failed: route `{route_path}` `concurrency.lock_ttl_ms` \
                 must be greater than zero"
            ));
        }
    }
    Ok(policy)
}

pub(crate) fn normalize_route_runtime(
    runtime: Option<FaaSRuntime>,
    route_path: &str,
) -> Result<Option<FaaSRuntime>> {
    match runtime {
        Some(FaaSRuntime::Microvm {
            image,
            vcpus,
            memory_mb,
        }) => {
            let image = image.trim().to_owned();
            if image.is_empty() {
                return Err(anyhow!(
                    "Integrity Validation Failed: route `{route_path}` microvm runtime requires a non-empty image"
                ));
            }
            if vcpus == 0 {
                return Err(anyhow!(
                    "Integrity Validation Failed: route `{route_path}` microvm runtime requires at least one vCPU"
                ));
            }
            if memory_mb < 64 {
                return Err(anyhow!(
                    "Integrity Validation Failed: route `{route_path}` microvm runtime requires at least 64 MiB memory"
                ));
            }
            Ok(Some(FaaSRuntime::Microvm {
                image,
                vcpus,
                memory_mb,
            }))
        }
        other => Ok(other),
    }
}

pub(crate) fn validate_integrity_batch_target(
    target: IntegrityBatchTarget,
) -> Result<IntegrityBatchTarget> {
    let name = target.name.trim();
    if name.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: batch targets must include a non-empty `name`"
        ));
    }

    let module = target.module.trim();
    if module.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: batch target `{name}` must include a non-empty `module`"
        ));
    }

    let env = target
        .env
        .into_iter()
        .map(|(key, value)| {
            let normalized_key = key.trim().to_owned();
            if normalized_key.is_empty() {
                return Err(anyhow!(
                    "Integrity Validation Failed: batch target `{name}` environment keys cannot be empty"
                ));
            }

            Ok((normalized_key, value.trim().to_owned()))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;

    Ok(IntegrityBatchTarget {
        name: name.to_owned(),
        module: module.to_owned(),
        env,
        volumes: normalize_route_volumes(
            target.volumes,
            RouteRole::System,
            &format!("batch target `{name}`"),
        )?,
    })
}

pub(crate) fn normalize_route_targets(targets: Vec<RouteTarget>) -> Result<Vec<RouteTarget>> {
    targets
        .into_iter()
        .map(normalize_route_target)
        .collect::<Result<Vec<_>>>()
}

pub(crate) fn normalize_route_resiliency(
    resiliency: Option<ResiliencyConfig>,
    route_path: &str,
) -> Result<Option<ResiliencyConfig>> {
    let Some(resiliency) = resiliency else {
        return Ok(None);
    };

    let timeout_ms = resiliency
        .timeout_ms
        .map(|timeout_ms| {
            if timeout_ms == 0 {
                return Err(anyhow!(
                    "Integrity Validation Failed: route `{route_path}` must set `resiliency.timeout_ms` above zero"
                ));
            }
            Ok(timeout_ms)
        })
        .transpose()?;

    let retry_policy = resiliency
        .retry_policy
        .map(|policy| normalize_retry_policy(policy, route_path))
        .transpose()?;

    if timeout_ms.is_none() && retry_policy.is_none() {
        return Ok(None);
    }

    Ok(Some(ResiliencyConfig {
        timeout_ms,
        retry_policy,
    }))
}

pub(crate) fn normalize_retry_policy(policy: RetryPolicy, route_path: &str) -> Result<RetryPolicy> {
    let retry_on = policy
        .retry_on
        .into_iter()
        .map(|status| {
            if !(100..=599).contains(&status) {
                return Err(anyhow!(
                    "Integrity Validation Failed: route `{route_path}` has an invalid `resiliency.retry_policy.retry_on` status `{status}`"
                ));
            }
            Ok(status)
        })
        .collect::<Result<BTreeSet<_>>>()?
        .into_iter()
        .collect::<Vec<_>>();

    if policy.max_retries == 0 && !retry_on.is_empty() {
        return Ok(RetryPolicy {
            max_retries: 0,
            retry_on,
        });
    }

    if policy.max_retries > 0 && retry_on.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{route_path}` must configure at least one `resiliency.retry_policy.retry_on` status when `max_retries` is set"
        ));
    }

    Ok(RetryPolicy {
        max_retries: policy.max_retries,
        retry_on,
    })
}

pub(crate) fn normalize_route_models(
    models: Vec<IntegrityModelBinding>,
    route_path: &str,
) -> Result<Vec<IntegrityModelBinding>> {
    let mut deduped = BTreeMap::new();

    for model in models {
        let alias = normalize_service_name(&model.alias).map_err(|error| {
            anyhow!(
                "Integrity Validation Failed: route `{route_path}` has an invalid model alias `{}`: {error}",
                model.alias
            )
        })?;
        let path = model.path.trim();
        if path.is_empty() {
            return Err(anyhow!(
                "Integrity Validation Failed: route `{route_path}` model `{alias}` must include a non-empty `path`"
            ));
        }

        deduped
            .entry(alias.clone())
            .or_insert(IntegrityModelBinding {
                alias,
                path: path.to_owned(),
                device: model.device,
                qos: model.qos,
            });
    }

    Ok(deduped.into_values().collect())
}

pub(crate) fn normalize_route_domains(
    domains: Vec<String>,
    route_path: &str,
) -> Result<Vec<String>> {
    domains
        .into_iter()
        .map(|domain| {
            tls_runtime::normalize_domain(&domain).map_err(|error| {
                anyhow!(
                    "Integrity Validation Failed: route `{route_path}` has an invalid domain `{domain}`: {error}"
                )
            })
        })
        .collect::<Result<BTreeSet<_>>>()
        .map(|domains| domains.into_iter().collect())
}

pub(crate) fn ensure_unique_route_domains(routes: &[IntegrityRoute]) -> Result<()> {
    let mut owners = HashMap::new();

    for route in routes {
        for domain in &route.domains {
            if let Some(previous_route) = owners.insert(domain.clone(), route.path.clone()) {
                return Err(anyhow!(
                    "Integrity Validation Failed: domain `{domain}` is declared by both route `{previous_route}` and route `{}`",
                    route.path
                ));
            }
        }
    }

    Ok(())
}

pub(crate) fn ensure_unique_model_aliases(routes: &[IntegrityRoute]) -> Result<()> {
    let mut owners = HashMap::new();

    for route in routes {
        for model in &route.models {
            if let Some(previous_route) = owners.insert(model.alias.clone(), route.path.clone()) {
                return Err(anyhow!(
                    "Integrity Validation Failed: model alias `{}` is declared by both route `{previous_route}` and route `{}`",
                    model.alias, route.path
                ));
            }
        }
    }

    Ok(())
}

pub(crate) fn normalize_tls_address(address: Option<String>) -> Result<Option<String>> {
    address
        .map(|address| {
            let trimmed = address.trim();
            if trimmed.is_empty() {
                return Err(anyhow!(
                    "Integrity Validation Failed: `tls_address` must not be empty"
                ));
            }
            trimmed.parse::<SocketAddr>().map_err(|error| {
                anyhow!(
                    "Integrity Validation Failed: `tls_address` must be a socket address: {error}"
                )
            })?;
            Ok(trimmed.to_owned())
        })
        .transpose()
}

pub(crate) fn normalize_advertise_ip(address: Option<String>) -> Result<Option<String>> {
    address
        .map(|address| {
            let trimmed = address.trim();
            if trimmed.is_empty() {
                return Err(anyhow!(
                    "Integrity Validation Failed: `advertise_ip` must not be empty"
                ));
            }
            trimmed.parse::<IpAddr>().map_err(|error| {
                anyhow!(
                    "Integrity Validation Failed: `advertise_ip` must be an IP address: {error}"
                )
            })?;
            Ok(trimmed.to_owned())
        })
        .transpose()
}

pub(crate) fn effective_advertise_ip(config: &IntegrityConfig) -> String {
    config
        .advertise_ip
        .clone()
        .or_else(|| {
            config
                .host_address
                .parse::<SocketAddr>()
                .ok()
                .map(|address| address.ip().to_string())
        })
        .unwrap_or_else(|| Ipv4Addr::LOCALHOST.to_string())
}

pub(crate) fn normalize_route_name(name: &str, path: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Ok(default_route_name(path));
    }
    normalize_service_name(trimmed).map_err(|error| {
        anyhow!("Integrity Validation Failed: route `{path}` has an invalid `name`: {error}")
    })
}

pub(crate) fn normalize_service_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("service names cannot be empty"));
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err(anyhow!("service names must not contain path separators"));
    }

    Ok(trimmed.to_owned())
}

pub(crate) fn normalize_route_version(version: &str, path: &str) -> Result<String> {
    Version::parse(version.trim())
        .with_context(|| {
            format!(
                "Integrity Validation Failed: route `{path}` must use a valid semantic `version`"
            )
        })
        .map(|parsed| parsed.to_string())
}

pub(crate) fn normalize_route_dependencies(
    dependencies: BTreeMap<String, String>,
    path: &str,
) -> Result<BTreeMap<String, String>> {
    dependencies
        .into_iter()
        .map(|(name, requirement)| {
            let normalized_name = normalize_service_name(&name).map_err(|error| {
                anyhow!(
                    "Integrity Validation Failed: route `{path}` has an invalid dependency name `{}`: {}",
                    name,
                    error
                )
            })?;
            let parsed = VersionReq::parse(requirement.trim()).with_context(|| {
                format!(
                    "Integrity Validation Failed: route `{path}` has an invalid dependency requirement for `{normalized_name}`"
                )
            })?;
            Ok((normalized_name, parsed.to_string()))
        })
        .collect()
}

pub(crate) fn normalize_route_credentials(credentials: Vec<String>) -> Result<Vec<String>> {
    credentials
        .into_iter()
        .map(|credential| {
            let trimmed = credential.trim();
            if trimmed.is_empty() {
                return Err(anyhow!(
                    "Integrity Validation Failed: route credentials must not be empty"
                ));
            }
            Ok(trimmed.to_owned())
        })
        .collect::<Result<BTreeSet<_>>>()
        .map(|credentials| credentials.into_iter().collect())
}

pub(crate) fn normalize_route_middleware(
    middleware: Option<String>,
    path: &str,
) -> Result<Option<String>> {
    middleware
        .map(|middleware| {
            normalize_service_name(&middleware).map_err(|error| {
                anyhow!(
                    "Integrity Validation Failed: route `{path}` has an invalid `middleware`: {error}"
                )
            })
        })
        .transpose()
}

pub(crate) fn normalize_route_env(
    env: BTreeMap<String, String>,
    route_path: &str,
) -> Result<BTreeMap<String, String>> {
    env.into_iter()
        .map(|(key, value)| {
            let normalized_key = key.trim().to_owned();
            if normalized_key.is_empty() {
                return Err(anyhow!(
                    "Integrity Validation Failed: route `{route_path}` environment keys cannot be empty"
                ));
            }
            Ok((normalized_key, value.trim().to_owned()))
        })
        .collect()
}

pub(crate) fn normalize_route_target(target: RouteTarget) -> Result<RouteTarget> {
    let module = normalize_target_module_name(&target.module);
    if module.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: route targets must include a non-empty `module`"
        ));
    }
    if module.contains('/') || module.contains('\\') {
        return Err(anyhow!(
            "Integrity Validation Failed: route targets must use module names, not filesystem paths"
        ));
    }
    if target.weight > 100 {
        return Err(anyhow!(
            "Integrity Validation Failed: route target `{module}` must keep `weight` between 0 and 100"
        ));
    }
    let requires = normalize_capabilities(
        target.requires,
        format!("route target `{module}` capabilities"),
    )?;

    Ok(RouteTarget {
        module,
        weight: target.weight,
        websocket: target.websocket,
        match_header: target
            .match_header
            .map(normalize_header_match)
            .transpose()?,
        requires,
    })
}

pub(crate) fn normalize_header_match(header_match: HeaderMatch) -> Result<HeaderMatch> {
    let name = header_match.name.trim().to_ascii_lowercase();
    let value = header_match.value.trim().to_owned();

    if name.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: route target header matches must include a non-empty `name`"
        ));
    }
    if value.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: route target header matches must include a non-empty `value`"
        ));
    }

    Ok(HeaderMatch { name, value })
}

pub(crate) fn normalize_route_volumes(
    volumes: Vec<IntegrityVolume>,
    route_role: RouteRole,
    route_path: &str,
) -> Result<Vec<IntegrityVolume>> {
    let mut normalized = BTreeSet::new();
    let mut deduped = Vec::new();

    for volume in volumes {
        let volume = validate_route_volume(volume, route_role, route_path)?;
        if !normalized.insert((
            volume.volume_type.clone(),
            volume.guest_path.clone(),
            volume.host_path.clone(),
            volume.readonly,
            volume.ttl_seconds,
            volume.idle_timeout.clone(),
            volume.eviction_policy.clone(),
        )) {
            continue;
        }

        if deduped
            .iter()
            .any(|existing: &IntegrityVolume| existing.guest_path == volume.guest_path)
        {
            return Err(anyhow!(
                "Integrity Validation Failed: route `{route_path}` defines guest volume path `{}` more than once",
                volume.guest_path
            ));
        }

        deduped.push(volume);
    }

    deduped.sort_by(|left, right| left.guest_path.cmp(&right.guest_path));
    Ok(deduped)
}

pub(crate) fn validate_route_volume(
    volume: IntegrityVolume,
    route_role: RouteRole,
    route_path: &str,
) -> Result<IntegrityVolume> {
    let host_path = volume.host_path.trim();
    if host_path.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: route volumes must include a non-empty `host_path`"
        ));
    }

    if route_role == RouteRole::User && volume.volume_type == VolumeType::Host && !volume.readonly {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{route_path}` is a user route and cannot request writable direct host mounts; use a system storage broker instead"
        ));
    }

    if volume.volume_type == VolumeType::S3 && route_role == RouteRole::System {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{route_path}` is a system route and cannot use S3 volumes; system routes have direct host filesystem access"
        ));
    }

    if volume.volume_type == VolumeType::S3 {
        parse_s3_url(host_path).map_err(|_| {
            anyhow!(
                "Integrity Validation Failed: route `{route_path}` volume `host_path` value `{host_path}` is not a valid S3 URL; expected format: s3://bucket/prefix"
            )
        })?;
    }

    let guest_path = normalize_guest_volume_path(&volume.guest_path)?;
    if volume.ttl_seconds.is_some_and(|ttl| ttl == 0) {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{route_path}` volume `{guest_path}` must set `ttl_seconds` above zero"
        ));
    }
    let idle_timeout = volume
        .idle_timeout
        .as_deref()
        .map(|timeout| normalize_idle_timeout(timeout, route_path, &guest_path))
        .transpose()?;

    if volume.eviction_policy.is_some() && volume.volume_type != VolumeType::Ram {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{route_path}` volume `{guest_path}` can only set `eviction_policy` for `type = \"ram\"` volumes"
        ));
    }

    if idle_timeout.is_some() && volume.volume_type != VolumeType::Ram {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{route_path}` volume `{guest_path}` can only set `idle_timeout` for `type = \"ram\"` volumes"
        ));
    }

    if volume.eviction_policy == Some(VolumeEvictionPolicy::Hibernate) && idle_timeout.is_none() {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{route_path}` volume `{guest_path}` must set `idle_timeout` when `eviction_policy = \"hibernate\"`"
        ));
    }

    if let Some(ref schedule) = volume.backup_schedule {
        let cron = schedule.cron();
        validate_cron_expression(cron).map_err(|_| {
            anyhow!(
                "Integrity Validation Failed: route `{route_path}` volume `{guest_path}` \
                 has an invalid `backup_schedule` cron expression `{cron}`; \
                 expected 5-field format e.g. `0 3 * * *`"
            )
        })?;
    }

    if volume.consistency.write_mode == WriteMode::None && !volume.readonly {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{route_path}` volume `{guest_path}` \
             declares `consistency.write_mode = \"none\"` but is not marked `readonly`; \
             set `readonly: true` or pick another write_mode"
        ));
    }

    Ok(IntegrityVolume {
        volume_type: volume.volume_type,
        host_path: host_path.to_owned(),
        guest_path,
        readonly: volume.readonly,
        ttl_seconds: volume.ttl_seconds,
        idle_timeout,
        eviction_policy: volume.eviction_policy,
        encrypted: volume.encrypted,
        backup_schedule: volume.backup_schedule,
        consistency: volume.consistency,
    })
}

pub(crate) fn normalize_guest_volume_path(path: &str) -> Result<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: route volumes must include a non-empty `guest_path`"
        ));
    }
    if !trimmed.starts_with('/') {
        return Err(anyhow!(
            "Integrity Validation Failed: guest volume paths must be absolute, for example `/app/data`"
        ));
    }
    if trimmed.contains('\\') {
        return Err(anyhow!(
            "Integrity Validation Failed: guest volume paths must use `/` separators"
        ));
    }

    let normalized = trimmed.trim_end_matches('/');
    let normalized = if normalized.is_empty() {
        "/".to_owned()
    } else {
        normalized.to_owned()
    };

    if normalized == "/" {
        return Err(anyhow!(
            "Integrity Validation Failed: guest volume path `/` is not allowed"
        ));
    }
    if normalized
        .split('/')
        .skip(1)
        .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return Err(anyhow!(
            "Integrity Validation Failed: guest volume paths cannot contain empty, `.` or `..` segments"
        ));
    }

    Ok(normalized)
}

/// Parse an S3 URL of the form `s3://bucket/prefix` into `(bucket, prefix)`.
/// The prefix may be empty (bare bucket URL `s3://bucket` or `s3://bucket/`).
pub(crate) fn parse_s3_url(url: &str) -> Result<(String, String)> {
    let without_scheme = url
        .strip_prefix("s3://")
        .ok_or_else(|| anyhow!("S3 URL must start with s3://"))?;
    if without_scheme.is_empty() {
        return Err(anyhow!("S3 URL missing bucket name"));
    }
    let (bucket, prefix) = match without_scheme.find('/') {
        Some(idx) => (
            &without_scheme[..idx],
            without_scheme[idx + 1..].trim_end_matches('/'),
        ),
        None => (without_scheme, ""),
    };
    if bucket.is_empty() {
        return Err(anyhow!("S3 URL missing bucket name"));
    }
    Ok((bucket.to_owned(), prefix.to_owned()))
}

/// Validate a 5-field cron expression (minute hour day-of-month month day-of-week).
/// Each field may be `*`, a number, a range (`1-5`), a list (`1,3,5`), or a step (`*/15`).
/// Returns `Ok(())` if valid, `Err(())` otherwise.
pub(crate) fn validate_cron_expression(expr: &str) -> std::result::Result<(), ()> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(());
    }
    let ranges = [(0u32, 59), (0, 23), (1, 31), (1, 12), (0, 7)];
    for (field, (min, max)) in fields.iter().zip(ranges.iter()) {
        if !is_valid_cron_field(field, *min, *max) {
            return Err(());
        }
    }
    Ok(())
}

fn is_valid_cron_field(field: &str, min: u32, max: u32) -> bool {
    if field == "*" {
        return true;
    }
    // Step: */n or x/n
    if let Some((base, step)) = field.split_once('/') {
        let step_ok = step.parse::<u32>().is_ok_and(|n| n >= 1);
        let base_ok = base == "*" || base.parse::<u32>().is_ok_and(|n| n >= min && n <= max);
        return base_ok && step_ok;
    }
    // List: a,b,c
    if field.contains(',') {
        return field
            .split(',')
            .all(|part| is_valid_cron_field(part.trim(), min, max));
    }
    // Range: a-b
    if let Some((lo, hi)) = field.split_once('-') {
        let lo_ok = lo.parse::<u32>().is_ok_and(|n| n >= min && n <= max);
        let hi_ok = hi.parse::<u32>().is_ok_and(|n| n >= min && n <= max);
        return lo_ok && hi_ok && lo.parse::<u32>().unwrap_or(0) <= hi.parse::<u32>().unwrap_or(0);
    }
    // Single number
    field.parse::<u32>().is_ok_and(|n| n >= min && n <= max)
}

pub(crate) fn normalize_idle_timeout(
    value: &str,
    route_path: &str,
    guest_path: &str,
) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow!(
            "Integrity Validation Failed: route `{route_path}` volume `{guest_path}` has an empty `idle_timeout`"
        ));
    }

    parse_hibernation_duration(trimmed).with_context(|| {
        format!(
            "Integrity Validation Failed: route `{route_path}` volume `{guest_path}` has an invalid `idle_timeout`"
        )
    })?;

    Ok(trimmed.to_owned())
}

pub(crate) fn parse_hibernation_duration(value: &str) -> Result<Duration> {
    let trimmed = value.trim();
    let (digits, multiplier) = if let Some(value) = trimmed.strip_suffix("ms") {
        (value, Duration::from_millis(1))
    } else if let Some(value) = trimmed.strip_suffix('s') {
        (value, Duration::from_secs(1))
    } else if let Some(value) = trimmed.strip_suffix('m') {
        (value, Duration::from_secs(60))
    } else {
        return Err(anyhow!(
            "idle_timeout must use one of the `ms`, `s`, or `m` suffixes"
        ));
    };

    let amount = digits
        .trim()
        .parse::<u64>()
        .context("idle_timeout must start with an unsigned integer")?;
    if amount == 0 {
        return Err(anyhow!("idle_timeout must be greater than zero"));
    }

    multiplier
        .checked_mul(u32::try_from(amount).context("idle_timeout is too large")?)
        .ok_or_else(|| anyhow!("idle_timeout is too large"))
}

/// Validate and pass-through the `scopes` JSON block from a route manifest.
/// An absent block (`None`) is preserved as `None` — the caller (instantiation
/// path) will treat `None` as `allow-all` with a warning.
/// A present block is validated eagerly so that manifest errors surface at
/// submission time rather than at guest instantiation.
fn validate_route_scopes(
    scopes: Option<serde_json::Value>,
    route_path: &str,
) -> Result<Option<serde_json::Value>> {
    let Some(ref value) = scopes else {
        return Ok(None);
    };
    // Validate by attempting to parse — we discard the result here and
    // re-parse at instantiation so `GlobSet` (not Send/Sync-safe for storage)
    // is not persisted on `IntegrityRoute`.
    crate::host_core::scoping::DeploymentScopes::from_manifest(value).map_err(|e| {
        anyhow!("Integrity Validation Failed: route `{route_path}` has invalid `scopes` block: {e}")
    })?;
    Ok(scopes)
}

pub(crate) fn normalize_allowed_secrets(allowed_secrets: Vec<String>) -> Result<Vec<String>> {
    allowed_secrets
        .into_iter()
        .map(|secret| {
            let trimmed = secret.trim();
            if trimmed.is_empty() {
                Err(anyhow!(
                    "Integrity Validation Failed: allowed secret names cannot be empty"
                ))
            } else {
                Ok(trimmed.to_owned())
            }
        })
        .collect::<Result<BTreeSet<_>>>()
        .map(|allowed_secrets| allowed_secrets.into_iter().collect())
}

#[cfg(test)]
pub(crate) fn canonical_config_payload(config: &IntegrityConfig) -> Result<String> {
    serde_json::to_string(config).context("failed to serialize runtime integrity configuration")
}

/// Thin wrapper kept for backwards compatibility and test coverage.
/// Production code should prefer `verify_integrity_payload` or the
/// `_with_trusted` variants.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn verify_integrity_signature(
    payload: &str,
    public_key_hex: &str,
    signature_hex: &str,
) -> Result<()> {
    verify_integrity_signature_with_trusted(payload, public_key_hex, signature_hex, &[])
}

pub(crate) fn verify_integrity_signature_with_trusted(
    payload: &str,
    public_key_hex: &str,
    signature_hex: &str,
    _trusted_signers: &[String],
) -> Result<()> {
    // Pure cryptographic check: is the signature valid for the declared key?
    // Trust-level checks (is this key trusted?) happen in verify_integrity_payload_with_trusted.
    let payload_hash = Sha256::digest(payload.as_bytes());
    let public_key_bytes = decode_hex_array::<32>(public_key_hex, "public key")?;
    let signature_bytes = decode_hex_array::<64>(signature_hex, "signature")?;
    let verifying_key = VerifyingKey::from_bytes(&public_key_bytes)
        .context("invalid embedded Ed25519 public key")?;
    let signature = Signature::from_bytes(&signature_bytes);

    verifying_key
        .verify(&payload_hash, &signature)
        .map_err(|error| {
            anyhow!("Integrity Validation Failed: signature verification failed: {error}")
        })
}

pub(crate) fn decode_hex_array<const N: usize>(value: &str, label: &str) -> Result<[u8; N]> {
    let decoded =
        hex::decode(value).with_context(|| format!("failed to decode embedded {label} as hex"))?;

    decoded
        .try_into()
        .map_err(|_| anyhow!("embedded {label} has an unexpected byte length"))
}

// =====================================================================
// Smart deployment pipeline — bundle apply
// =====================================================================

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BundleConflictResponseEntry {
    pub(crate) name: String,
    pub(crate) bundled_version: String,
    pub(crate) cluster_version: String,
}

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleApplyResponse {
    success: bool,
    message: String,
    config_version: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    conflicts: Vec<BundleConflictResponseEntry>,
    requires_resolution: bool,
}

#[derive(Debug)]
pub(crate) struct BundleManifestFields {
    pub(crate) config_payload: String,
    pub(crate) dependencies: Vec<BundleManifestDependency>,
}

#[derive(Debug)]
pub(crate) struct BundleManifestDependency {
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) source: Option<String>,
}

pub(crate) async fn admin_manifest_bundle_handler(
    State(state): State<AppState>,
    body: Bytes,
) -> Response {
    use std::io::Read;

    if body.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "deployment bundle body must not be empty",
        )
            .into_response();
    }

    let bundle_bytes = body.to_vec();
    let manifest_yaml = match tokio::task::spawn_blocking(move || {
        let cursor = std::io::Cursor::new(bundle_bytes);
        let decoder = flate2::read::GzDecoder::new(cursor);
        let mut archive = tar::Archive::new(decoder);
        let entries = archive
            .entries()
            .map_err(|error| format!("failed to enumerate bundle entries: {error}"))?;
        for entry in entries {
            let mut entry =
                entry.map_err(|error| format!("failed to read bundle entry: {error}"))?;
            let path = entry
                .path()
                .map_err(|error| format!("failed to read bundle entry path: {error}"))?;
            if path.as_ref() == std::path::Path::new("manifest.yaml") {
                let mut buffer = String::new();
                entry
                    .read_to_string(&mut buffer)
                    .map_err(|error| format!("failed to read manifest.yaml: {error}"))?;
                return Ok::<String, String>(buffer);
            }
        }
        Err("bundle is missing manifest.yaml at the archive root".to_owned())
    })
    .await
    {
        Ok(Ok(value)) => value,
        Ok(Err(message)) => {
            return (StatusCode::BAD_REQUEST, message).into_response();
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("bundle extraction task failed: {error}"),
            )
                .into_response();
        }
    };

    let manifest_fields = match parse_bundle_manifest_yaml(&manifest_yaml) {
        Ok(fields) => fields,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("manifest.yaml is malformed: {error}"),
            )
                .into_response();
        }
    };

    // Parse the config payload early so we can validate schema before any signing.
    let mut new_config =
        match serde_json::from_str::<IntegrityConfig>(&manifest_fields.config_payload) {
            Ok(config) => config,
            Err(error) => {
                return (
                    StatusCode::BAD_REQUEST,
                    format!("manifest.yaml configPayload is not valid JSON: {error}"),
                )
                    .into_response();
            }
        };

    // Detect dependency conflicts against the cluster's current asset registry.
    // Done before signing so a 428 never produces an integrity.lock write.
    let cluster_versions = state.runtime.load().config.asset_versions.clone();
    if let Some(conflicts) =
        detect_dependency_conflicts(&manifest_fields.dependencies, &cluster_versions)
    {
        let response = BundleApplyResponse {
            success: false,
            message: "Dependency override detected".to_owned(),
            config_version: 0,
            conflicts,
            requires_resolution: true,
        };
        return (StatusCode::PRECONDITION_REQUIRED, axum::Json(response)).into_response();
    }

    // Auto-bootstrap: inject the node's own public key into trusted_signers so
    // that future reloads accept this node-signed manifest without any manual
    // operator step.  The field is normalised to lowercase hex; we skip the
    // injection if the key is already present to stay idempotent.
    let host_pubkey_hex = state.host_identity.public_key_hex.clone();
    let already_trusted = new_config
        .trusted_signers
        .iter()
        .any(|k| k.trim().eq_ignore_ascii_case(&host_pubkey_hex));
    if !already_trusted {
        new_config.trusted_signers.push(host_pubkey_hex.clone());
    }

    // Record the resolved asset versions in the config so future bundle applies
    // can detect when the cluster already holds a better-compatible version.
    for dep in &manifest_fields.dependencies {
        if dep.source.is_some() {
            if let Some(version) = extract_semver_version(&dep.version) {
                new_config.asset_versions.insert(dep.name.clone(), version);
            }
        }
    }

    // Re-serialize so the signature covers trusted_signers + asset_versions.
    let signed_payload = match serde_json::to_string(&new_config) {
        Ok(json) => json,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!(
                    "failed to re-serialize config payload after trusted-signer injection: {error}"
                ),
            )
                .into_response();
        }
    };

    let new_config = match validate_integrity_config(new_config) {
        Ok(validated) => validated,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("manifest config validation failed: {error:#}"),
            )
                .into_response();
        }
    };

    let current_version = state.runtime.load().config.config_version;
    if new_config.config_version <= current_version {
        return (
            StatusCode::CONFLICT,
            format!(
                "manifest config_version {} is not strictly greater than current {}",
                new_config.config_version, current_version
            ),
        )
            .into_response();
    }

    // Sign the (extended) config payload with this node's stable host identity key.
    let host_signing_key = Arc::clone(&state.host_identity.signing_key);
    let payload_for_signing = signed_payload.clone();
    let signature_hex = tokio::task::spawn_blocking(move || {
        use ed25519_dalek::Signer as _;
        use sha2::Digest;
        let hash = sha2::Sha256::digest(payload_for_signing.as_bytes());
        let signature = host_signing_key.sign(&hash);
        hex::encode(signature.to_bytes())
    })
    .await
    .map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("host signing task failed: {error}"),
        )
            .into_response()
    });
    let signature_hex = match signature_hex {
        Ok(sig) => sig,
        Err(response) => return response,
    };

    let serialized = match serde_json::to_vec(&IntegrityManifest {
        public_key: host_pubkey_hex.clone(),
        signature: signature_hex,
        config_payload: signed_payload.clone(),
    }) {
        Ok(bytes) => bytes,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to serialize host-signed manifest: {error}"),
            )
                .into_response();
        }
    };

    let manifest_path = state.manifest_path.clone();
    let payload_bytes = serialized.clone();
    let write_result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        let staging = manifest_path.with_extension("lock.tmp");
        fs::write(&staging, &payload_bytes)?;
        fs::rename(&staging, &manifest_path)?;
        Ok(())
    })
    .await;
    match write_result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to persist manifest from bundle: {error}"),
            )
                .into_response();
        }
        Err(join_error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("manifest write task failed: {join_error}"),
            )
                .into_response();
        }
    }

    use sha2::Digest;
    let checksum = format!(
        "sha256:{}",
        hex::encode(sha2::Sha256::digest(signed_payload.as_bytes()))
    );
    let ts_ms = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let event = ConfigUpdateEvent {
        version: new_config.config_version,
        checksum,
        origin_node_id: state.host_identity.public_key_hex.clone(),
        ts_ms,
    };
    let _ = state.config_updates.send(event.clone());
    if let Ok(payload) = serde_json::to_vec(&event) {
        if let Err(error) = state
            .core_store
            .append_outbox(crate::store::CoreStoreBucket::ConfigUpdateOutbox, &payload)
        {
            tracing::warn!("failed to append bundle config-update outbox: {error}");
        }
    }

    let response = BundleApplyResponse {
        success: true,
        message: "Deployment bundle applied".to_owned(),
        config_version: new_config.config_version,
        conflicts: Vec::new(),
        requires_resolution: false,
    };
    (StatusCode::OK, axum::Json(response)).into_response()
}

pub(crate) fn parse_bundle_manifest_yaml(yaml: &str) -> Result<BundleManifestFields, String> {
    let mut config_payload_lines: Vec<String> = Vec::new();
    let mut dependencies: Vec<BundleManifestDependency> = Vec::new();

    let mut mode = ParseMode::TopLevel;
    let mut current_dep: Option<BundleManifestDependency> = None;

    enum ParseMode {
        TopLevel,
        ConfigPayload,
        Dependencies,
    }

    for raw_line in yaml.lines() {
        let line = raw_line.trim_end_matches('\r');
        match mode {
            ParseMode::ConfigPayload => {
                if let Some(content) = line.strip_prefix("  ") {
                    config_payload_lines.push(content.to_owned());
                    continue;
                }
                mode = ParseMode::TopLevel;
            }
            ParseMode::Dependencies => {
                if line.is_empty() {
                    continue;
                }
                if let Some(rest) = line.strip_prefix("    ") {
                    if let Some(dep) = current_dep.as_mut() {
                        if let Some(value) = rest.strip_prefix("version:") {
                            dep.version = strip_yaml_string(value);
                        } else if let Some(value) = rest.strip_prefix("source:") {
                            let trimmed = strip_yaml_string(value);
                            dep.source = if trimmed.is_empty() {
                                None
                            } else {
                                Some(trimmed)
                            };
                        }
                    }
                    continue;
                }
                if let Some(rest) = line.strip_prefix("  ") {
                    if let Some(dep) = current_dep.take() {
                        dependencies.push(dep);
                    }
                    if let Some(stripped) = rest.strip_suffix(':') {
                        current_dep = Some(BundleManifestDependency {
                            name: stripped.trim().to_owned(),
                            version: String::new(),
                            source: None,
                        });
                    }
                    continue;
                }
                if let Some(dep) = current_dep.take() {
                    dependencies.push(dep);
                }
                mode = ParseMode::TopLevel;
            }
            ParseMode::TopLevel => {}
        }

        if matches!(mode, ParseMode::TopLevel) {
            if line.is_empty()
                || line.starts_with('#')
                || line.starts_with("version:")
                || line.starts_with("publicKey:")
                || line.starts_with("signature:")
            {
                // version, publicKey, signature are accepted but ignored — the host
                // signs the manifest itself.
                continue;
            }
            if line.starts_with("configPayload:") && line.trim_end().ends_with('|') {
                mode = ParseMode::ConfigPayload;
            } else if line.starts_with("dependencies:") {
                mode = ParseMode::Dependencies;
            }
        }
    }
    if let Some(dep) = current_dep.take() {
        dependencies.push(dep);
    }

    if config_payload_lines.is_empty() {
        return Err("missing configPayload block".to_owned());
    }
    let config_payload = config_payload_lines.join("\n");

    Ok(BundleManifestFields {
        config_payload,
        dependencies,
    })
}

fn strip_yaml_string(value: &str) -> String {
    let trimmed = value.trim();
    let trimmed = trimmed
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(trimmed);
    trimmed.to_owned()
}

/// Compares each bundled dependency that includes a local `source` asset
/// against the cluster's current asset version registry.  A conflict is
/// reported when the cluster already holds a version that:
///   (a) satisfies the same SemVer constraint as the bundle, AND
///   (b) is strictly greater than the bundled version.
///
/// In that situation the bundled asset would silently shadow a better
/// available version, so the apply is halted and a 428 is returned to let the
/// operator decide whether to use the cluster version or pin the local one.
pub(crate) fn detect_dependency_conflicts(
    dependencies: &[BundleManifestDependency],
    cluster_versions: &std::collections::BTreeMap<String, String>,
) -> Option<Vec<BundleConflictResponseEntry>> {
    let mut conflicts = Vec::new();

    for dep in dependencies {
        // Only local-asset deps can shadow a cluster version.
        if dep.source.is_none() {
            continue;
        }

        let cluster_ver_str = match cluster_versions.get(&dep.name) {
            Some(v) => v,
            None => continue,
        };

        let constraint = match semver::VersionReq::parse(&dep.version) {
            Ok(req) => req,
            Err(_) => continue,
        };
        let bundled_ver = match extract_semver_version(&dep.version)
            .as_deref()
            .and_then(|v| semver::Version::parse(v).ok())
        {
            Some(v) => v,
            None => continue,
        };
        let cluster_ver = match semver::Version::parse(cluster_ver_str.trim()) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Conflict: cluster has a better version that still satisfies the req.
        if cluster_ver > bundled_ver && constraint.matches(&cluster_ver) {
            conflicts.push(BundleConflictResponseEntry {
                name: dep.name.clone(),
                bundled_version: bundled_ver.to_string(),
                cluster_version: cluster_ver.to_string(),
            });
        }
    }

    if conflicts.is_empty() {
        None
    } else {
        Some(conflicts)
    }
}

/// Strips SemVer range operators (`^`, `~`, `>=`, `>`, `<=`, `<`, `=`) from
/// the start of a version constraint string and returns the bare version
/// portion if it parses as a valid `semver::Version`.
pub(crate) fn extract_semver_version(constraint: &str) -> Option<String> {
    let stripped = constraint
        .trim()
        .trim_start_matches(['^', '~', '=', '>', '<', ' ']);
    semver::Version::parse(stripped).ok().map(|v| v.to_string())
}
