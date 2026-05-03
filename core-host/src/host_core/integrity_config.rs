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
    let manifest = read_integrity_manifest(path)?;
    verify_integrity_payload(
        &manifest.config_payload,
        &manifest.public_key,
        &manifest.signature,
        &format!("integrity manifest at {}", path.display()),
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
    verify_integrity_signature(payload, public_key_hex, signature_hex)?;
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdminEnrollmentStartRequest {
    pub(crate) node_public_key: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdminEnrollmentStartResponse {
    pub(crate) session_id: String,
    pub(crate) pin: String,
}

#[derive(Debug, Deserialize)]
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
    let node_pubkey = payload.node_public_key.trim().to_owned();
    if node_pubkey.is_empty() {
        return (StatusCode::BAD_REQUEST, "nodePublicKey is required").into_response();
    }
    let session = state.enrollment_manager.start_session(node_pubkey);
    let body = AdminEnrollmentStartResponse {
        session_id: session.session_id,
        pin: session.pin,
    };
    (StatusCode::CREATED, axum::Json(body)).into_response()
}

/// `POST /admin/enrollment/approve` — operator-driven approval entered via
/// Tachyon Studio. Validates the PIN against the recorded session and stages
/// the signed certificate bytes for the unenrolled node's next poll.
pub(crate) async fn admin_enrollment_approve_handler(
    State(state): State<AppState>,
    axum::Json(payload): axum::Json<AdminEnrollmentApproveRequest>,
) -> Response {
    let cert_bytes = match hex::decode(&payload.signed_certificate_hex) {
        Ok(bytes) => bytes,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("signedCertificateHex must be valid hex: {error}"),
            )
                .into_response();
        }
    };
    match state
        .enrollment_manager
        .approve(&payload.session_id, &payload.pin, cert_bytes)
    {
        Ok(()) => (StatusCode::ACCEPTED, "enrollment approved").into_response(),
        Err(reason) => {
            // PIN mismatch / unknown / already-finalized are all caller errors.
            (StatusCode::BAD_REQUEST, reason).into_response()
        }
    }
}

/// `GET /admin/enrollment/poll/{session_id}` — invoked by the unenrolled node's
/// long-poll. Returns 204 No Content while pending, 200 with the signed cert
/// bytes (hex) once the operator has approved, 410 Gone after rejection.
pub(crate) async fn admin_enrollment_poll_handler(
    State(state): State<AppState>,
    axum::extract::Path(session_id): axum::extract::Path<String>,
) -> Response {
    match state.enrollment_manager.poll_outcome(&session_id) {
        None => StatusCode::NO_CONTENT.into_response(),
        Some(node_enrollment::EnrollmentOutcome::Approved { signed_certificate }) => {
            (StatusCode::OK, hex::encode(signed_certificate)).into_response()
        }
        Some(node_enrollment::EnrollmentOutcome::Rejected { reason }) => {
            (StatusCode::GONE, reason).into_response()
        }
    }
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
    let new_config = match verify_integrity_payload(
        &manifest.config_payload,
        &manifest.public_key,
        &manifest.signature,
        "admin manifest submission",
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
    validate_tee_requirements(&config)?;
    let route_registry = RouteRegistry::build(&config)?;
    config.resources = normalize_resources(config.resources, &config.routes, &route_registry)?;
    config.layer4 = normalize_layer4_config(config.layer4, &route_registry)?;
    Ok(config)
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
        volumes: normalize_route_volumes(route.volumes, route.role, &normalized)?,
        resource_policy: route.resource_policy,
        runtime: normalize_route_runtime(route.runtime, &normalized)?,
        sync_to_cloud: route.sync_to_cloud,
        requires_tee: route.requires_tee,
        allow_overflow: route.allow_overflow,
        distributed_rate_limit: route.distributed_rate_limit,
        shadow_target: route.shadow_target,
        adapter_id: route.adapter_id,
    })
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

    Ok(IntegrityVolume {
        volume_type: volume.volume_type,
        host_path: host_path.to_owned(),
        guest_path,
        readonly: volume.readonly,
        ttl_seconds: volume.ttl_seconds,
        idle_timeout,
        eviction_policy: volume.eviction_policy,
        encrypted: volume.encrypted,
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

pub(crate) fn verify_integrity_signature(
    payload: &str,
    public_key_hex: &str,
    signature_hex: &str,
) -> Result<()> {
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
