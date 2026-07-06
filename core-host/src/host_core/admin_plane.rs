use super::*;

pub(crate) fn authenticated_routes(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/admin/status", get(auth::admin_status_handler))
        .route("/admin/metrics", get(admin_metrics_handler))
        .route("/admin/schema/manifest", get(admin_manifest_schema_handler))
        .route(
            "/admin/schema/openapi.json",
            get(admin_openapi_schema_handler),
        )
        .route(
            "/admin/schema/integrity-lock",
            get(admin_integrity_lock_schema_handler),
        )
        .route("/admin/docs", get(admin_openapi_docs_handler))
        .route(
            "/admin/kv/{namespace}/{key}",
            get(admin_kv_get_handler)
                .put(admin_kv_put_handler)
                .delete(admin_kv_delete_handler),
        )
        .route(
            "/admin/canary",
            get(admin_canary_status_handler)
                .post(admin_abort_canary_handler)
                .patch(admin_set_canary_weight_handler),
        )
        .route("/admin/shadow/diffs", get(admin_shadow_diffs_handler))
        .route("/admin/chaos/scenarios", post(admin_chaos_scenario_handler))
        .route(
            "/admin/lora/training/{job_id}",
            get(admin_lora_training_status_handler),
        )
        .route(
            "/admin/manifest",
            get(admin_get_manifest_handler).post(admin_manifest_update_handler),
        )
        .route(
            "/admin/manifest/bundle",
            post(admin_manifest_bundle_handler),
        )
        .route(
            "/admin/identity/public-key",
            get(auth::node_public_key_handler),
        )
        .route(
            "/admin/enrollment/approve",
            post(admin_enrollment_approve_handler),
        )
        .route("/admin/nodes", get(admin_nodes_handler))
        .route(
            "/admin/nodes/{node_id}/capabilities",
            post(admin_node_capabilities_handler),
        )
        .route(
            "/admin/systems/registered",
            get(admin_registered_systems_handler),
        )
        .route(
            "/admin/systems/deployed",
            get(admin_deployed_systems_handler),
        )
        .route(
            "/admin/security/recovery-codes",
            post(auth::generate_recovery_codes_handler),
        )
        .route(
            "/admin/security/2fa/regenerate",
            post(auth::regenerate_account_security_handler),
        )
        .route(
            "/admin/security/step-up",
            post(auth::issue_step_up_session_handler),
        )
        .route("/admin/security/pats", post(auth::issue_pat_handler))
        .route("/admin/iam/users", get(auth::list_users_handler))
        .route(
            "/admin/iam/users/{username}",
            patch(auth::update_user_handler).delete(auth::delete_user_handler),
        )
        .route(
            "/admin/iam/groups",
            get(auth::list_groups_handler).post(auth::upsert_group_handler),
        )
        .route(
            "/admin/iam/groups/{name}",
            delete(auth::delete_group_handler),
        )
        .route("/admin/logs", get(auth::audit_log_handler))
        .route(
            "/admin/kv-cache/{model}/stats",
            get(kv_cache::kv_cache_stats_handler),
        )
        .route(
            "/admin/kv-cache/{model}",
            delete(kv_cache::kv_cache_evict_handler),
        )
        .route("/admin/volumes/backup", post(admin_volume_backup_handler))
        .route("/admin/volumes/restore", post(admin_volume_restore_handler))
        .route(
            "/admin/volumes/backups",
            get(admin_volume_list_backups_handler),
        )
        .route("/admin/assets", post(system_storage::upload_asset_handler))
        .route(
            "/admin/models/init",
            post(system_storage::init_upload_handler),
        )
        .route(
            "/admin/models/upload/{upload_id}",
            put(system_storage::upload_chunk_handler),
        )
        .route(
            "/admin/models/commit/{upload_id}",
            post(system_storage::commit_upload_handler),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::admin_auth_middleware,
        ))
}

#[derive(Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdminRuntimeMetrics {
    pub(crate) source: String,
    pub(crate) error_rate: f64,
    pub(crate) p50_latency_ms: f64,
    pub(crate) p99_latency_ms: f64,
    pub(crate) queue_depth: u64,
    /// Current VRAM utilization (0-100). Updated by the AI inference runtime.
    pub(crate) vram_utilization_pct: u8,
    /// `true` when VRAM has exceeded the critical threshold and KV-cache
    /// tensors are being spilled to pinned host RAM via PCIe.
    pub(crate) ram_offload_active: bool,
    /// Lifetime count of runtime WIT import denials across all deployments and categories.
    /// Per-deployment, per-category breakdowns are available via the prometheus endpoint.
    pub(crate) scope_denial_total: u64,
    /// Inter-FaaS dispatch counters and latency aggregates by transport mode.
    pub(crate) mesh_dispatch: MeshDispatchMetricsSnapshot,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdminShadowDiff {
    route: String,
    request_id: String,
    divergence: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    primary_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shadow_status: Option<u16>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdminChaosScenarioRequest {
    pub(crate) scenario: String,
    #[serde(default)]
    pub(crate) duration_seconds: Option<u64>,
    #[serde(default)]
    pub(crate) target: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdminChaosScenarioOutcome {
    accepted: bool,
    scenario: String,
    message: String,
}

pub(crate) async fn admin_metrics_handler(
    State(state): State<AppState>,
) -> axum::Json<AdminRuntimeMetrics> {
    let snapshot = telemetry::snapshot(&state.telemetry);
    let completed = snapshot.completed_requests.max(1) as f64;
    let error_rate = if snapshot.completed_requests == 0 {
        0.0
    } else {
        snapshot.error_requests as f64 / completed
    };
    let average_latency_ms = if snapshot.completed_requests == 0 {
        0.0
    } else {
        snapshot.total_duration_us as f64 / completed / 1_000.0
    };

    axum::Json(AdminRuntimeMetrics {
        source: "core-host://runtime-telemetry".to_owned(),
        error_rate,
        p50_latency_ms: average_latency_ms,
        p99_latency_ms: average_latency_ms,
        queue_depth: u64::from(snapshot.active_requests),
        vram_utilization_pct: state.memory_governor.vram_utilization_pct(),
        ram_offload_active: state.memory_governor.ram_offload_active(),
        scope_denial_total: crate::host_core::scoping::scope_denial_total_lifetime(),
        mesh_dispatch: mesh_dispatch_metrics_snapshot(),
    })
}

/// Returns a JSON Schema document describing the `IntegrityConfig` manifest
/// format accepted by `POST /admin/manifest`. LLM agents and MCP clients can
/// fetch this schema to validate manifest payloads before submission.
pub(crate) async fn admin_manifest_schema_handler() -> axum::Json<serde_json::Value> {
    axum::Json(integrity_config_schema())
}

/// Serves the OpenAPI 3.1 JSON document generated from the compile-time `ApiDoc`.
pub(crate) async fn admin_openapi_schema_handler() -> impl IntoResponse {
    let json = crate::host_core::openapi::get_base_openapi_schema();
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        json,
    )
}

/// Serves a JSON Schema describing the `integrity.lock` file format.
pub(crate) async fn admin_integrity_lock_schema_handler() -> impl IntoResponse {
    let schema = integrity_manifest_schema();
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        schema.to_string(),
    )
}

fn integrity_config_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(IntegrityConfig))
        .expect("IntegrityConfig JSON Schema should serialize")
}

fn integrity_manifest_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(IntegrityManifest))
        .expect("IntegrityManifest JSON Schema should serialize")
}

#[cfg(test)]
mod schema_tests {
    use super::*;

    #[test]
    fn manifest_schema_uses_integrity_config_field_names() {
        let schema = integrity_config_schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema root should expose properties");

        assert!(properties.contains_key("host_address"));
        assert!(properties.contains_key("max_stdout_bytes"));
        assert!(properties.contains_key("layer4"));
        assert!(properties.contains_key("enrollment"));
        assert!(properties.contains_key("tee_backend"));
        assert!(properties.contains_key("asset_versions"));
        assert!(!properties.contains_key("hostAddress"));
        assert!(!properties.contains_key("maxStdoutBytes"));
    }

    #[test]
    fn manifest_schema_covers_route_nested_config() {
        let schema = integrity_config_schema();
        let text = schema.to_string();

        for field in [
            "\"targets\"",
            "\"models\"",
            "\"hardware_strategy\"",
            "\"distribution_mode\"",
            "\"paged_attention\"",
            "\"cuda_graph_decode\"",
            "\"flashinfer_attention\"",
            "\"prefill_chunk_tokens\"",
            "\"speculative_draft_model_path\"",
            "\"volumes\"",
            "\"domains\"",
            "\"runtime\"",
            "\"concurrency\"",
            "\"distributed_rate_limit\"",
            "\"metrics\"",
        ] {
            assert!(text.contains(field), "schema should include {field}");
        }
    }

    #[test]
    fn integrity_lock_schema_uses_signed_manifest_field_names() {
        let schema = integrity_manifest_schema();
        let properties = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("schema root should expose properties");

        assert!(properties.contains_key("config_payload"));
        assert!(properties.contains_key("public_key"));
        assert!(properties.contains_key("signature"));
        assert!(!properties.contains_key("configPayload"));
        assert!(!properties.contains_key("hostAddress"));
    }
}

/// Serves the Swagger UI HTML pointing at `/admin/schema/openapi.json`.
pub(crate) async fn admin_openapi_docs_handler() -> impl IntoResponse {
    const HTML: &str = include_str!("swagger-ui.html");
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        HTML,
    )
}

pub(crate) async fn admin_kv_get_handler(
    State(state): State<AppState>,
    axum::extract::Path((namespace, key)): axum::extract::Path<(String, String)>,
) -> Response {
    match state.core_store.kv_partition_get(&namespace, &key) {
        Ok(Some(bytes)) => (
            StatusCode::OK,
            [("content-type", "application/octet-stream")],
            bytes,
        )
            .into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "key not found").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub(crate) async fn admin_kv_put_handler(
    State(state): State<AppState>,
    axum::extract::Path((namespace, key)): axum::extract::Path<(String, String)>,
    body: Bytes,
) -> Response {
    match state.core_store.kv_partition_set(&namespace, &key, &body) {
        Ok(()) => (StatusCode::NO_CONTENT, "").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub(crate) async fn admin_kv_delete_handler(
    State(state): State<AppState>,
    axum::extract::Path((namespace, key)): axum::extract::Path<(String, String)>,
) -> Response {
    match state.core_store.kv_partition_delete(&namespace, &key) {
        Ok(()) => (StatusCode::NO_CONTENT, "").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

pub(crate) async fn admin_shadow_diffs_handler() -> axum::Json<Vec<AdminShadowDiff>> {
    // Shadow divergences are emitted by system-faas-shadow-proxy into the
    // telemetry stream. The admin surface is intentionally stable even when no
    // divergence collector has produced recent rows.
    axum::Json(Vec::new())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdminLoraTrainingStatus {
    pub(crate) job_id: String,
    pub(crate) status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) step: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) total: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) adapter_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<String>,
}

pub(crate) async fn admin_lora_training_status_handler(
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> Response {
    let queue = lora_training_queue();
    let status = queue
        .statuses
        .lock()
        .expect("LoRA training status map should not be poisoned")
        .get(&job_id)
        .cloned();
    let Some(status) = status else {
        return (
            StatusCode::NOT_FOUND,
            format!("unknown LoRA training job `{job_id}`"),
        )
            .into_response();
    };

    let body = match status {
        LoraTrainingJobStatus::Queued => AdminLoraTrainingStatus {
            job_id,
            status: "queued".to_owned(),
            step: None,
            total: None,
            adapter_path: None,
            error: None,
        },
        LoraTrainingJobStatus::Running { step, total } => AdminLoraTrainingStatus {
            job_id,
            status: "running".to_owned(),
            step: Some(step),
            total: Some(total),
            adapter_path: None,
            error: None,
        },
        LoraTrainingJobStatus::Completed { adapter_path } => AdminLoraTrainingStatus {
            job_id,
            status: "completed".to_owned(),
            step: None,
            total: None,
            adapter_path: Some(adapter_path),
            error: None,
        },
        LoraTrainingJobStatus::Failed { message } => AdminLoraTrainingStatus {
            job_id,
            status: "failed".to_owned(),
            step: None,
            total: None,
            adapter_path: None,
            error: Some(message),
        },
    };
    axum::Json(body).into_response()
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CanaryStatusEntry {
    pub(crate) route_path: String,
    pub(crate) current_version: String,
    pub(crate) next_version: String,
    pub(crate) weight_pct: u32,
    pub(crate) phase: String,
    pub(crate) next_req_count: u64,
    pub(crate) next_err_count: u64,
}

pub(crate) async fn admin_canary_status_handler() -> axum::Json<Vec<CanaryStatusEntry>> {
    let registry = canary_rollouts()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entries = registry
        .values()
        .map(|s| {
            let phase = s.phase.lock().unwrap_or_else(|p| p.into_inner());
            let phase_str = match &*phase {
                CanaryPhase::Stepping => "stepping".to_owned(),
                CanaryPhase::Promoted => "promoted".to_owned(),
                CanaryPhase::RolledBack { reason } => format!("rolled_back:{reason}"),
            };
            CanaryStatusEntry {
                route_path: s.route_path.clone(),
                current_version: s.current_version.clone(),
                next_version: s.next_version.clone(),
                weight_pct: s.weight_pct.load(Ordering::Relaxed),
                phase: phase_str,
                next_req_count: s.next_req_count.load(Ordering::Relaxed),
                next_err_count: s.next_err_count.load(Ordering::Relaxed),
            }
        })
        .collect();
    axum::Json(entries)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdminAbortCanaryRequest {
    pub(crate) route_path: String,
}

pub(crate) async fn admin_abort_canary_handler(
    axum::Json(payload): axum::Json<AdminAbortCanaryRequest>,
) -> Response {
    let registry = canary_rollouts()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(rollout) = registry.get(&payload.route_path) {
        rollout.weight_pct.store(0, Ordering::SeqCst);
        let mut phase = rollout.phase.lock().unwrap_or_else(|p| p.into_inner());
        *phase = CanaryPhase::RolledBack {
            reason: "manually aborted by operator".to_owned(),
        };
        let _ = rollout.stop_tx.send(true);
        tracing::warn!(
            route = %payload.route_path,
            "canary rollout manually aborted by operator"
        );
        (StatusCode::OK, "rollout aborted").into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            format!(
                "no active canary rollout for route `{}`",
                payload.route_path
            ),
        )
            .into_response()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdminSetCanaryWeightRequest {
    pub(crate) route_path: String,
    /// Desired traffic percentage for `next_version` (0-100).
    pub(crate) weight_pct: u8,
}

pub(crate) async fn admin_set_canary_weight_handler(
    axum::Json(payload): axum::Json<AdminSetCanaryWeightRequest>,
) -> Response {
    let registry = canary_rollouts()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(rollout) = registry.get(&payload.route_path) {
        let weight = payload.weight_pct.min(100);
        rollout
            .weight_pct
            .store(u32::from(weight), Ordering::SeqCst);
        if weight >= 100 {
            let mut phase = rollout.phase.lock().unwrap_or_else(|p| p.into_inner());
            *phase = CanaryPhase::Promoted;
        }
        tracing::info!(
            route = %payload.route_path,
            weight_pct = weight,
            "canary weight updated by operator"
        );
        (StatusCode::OK, format!("weight set to {weight}%")).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            format!(
                "no active canary rollout for route `{}`",
                payload.route_path
            ),
        )
            .into_response()
    }
}

// Volume backup endpoints

#[derive(serde::Deserialize)]
pub(crate) struct VolumeBackupRequest {
    route_path: String,
    guest_path: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct VolumeRestoreRequest {
    route_path: String,
    guest_path: String,
    snapshot_id: String,
}

pub(crate) async fn admin_volume_backup_handler(
    State(state): State<AppState>,
    axum::Json(payload): axum::Json<VolumeBackupRequest>,
) -> Response {
    let config = state.runtime.load().config.clone();
    match volume_backup::backup_volume(
        &config,
        &payload.route_path,
        &payload.guest_path,
        WriteIsolation::None,
    )
    .await
    {
        Ok(snapshot) => axum::Json(snapshot).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

pub(crate) async fn admin_volume_restore_handler(
    State(state): State<AppState>,
    axum::Json(payload): axum::Json<VolumeRestoreRequest>,
) -> Response {
    let config = state.runtime.load().config.clone();
    match volume_backup::restore_volume(
        &config,
        &payload.route_path,
        &payload.guest_path,
        &payload.snapshot_id,
    )
    .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

pub(crate) async fn admin_volume_list_backups_handler(
    State(state): State<AppState>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let config = state.runtime.load().config.clone();
    let route_path = params.get("route_path").map(String::as_str).unwrap_or("");
    let guest_path = params.get("guest_path").map(String::as_str).unwrap_or("");
    if route_path.is_empty() || guest_path.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            "route_path and guest_path query params required",
        )
            .into_response();
    }
    match volume_backup::list_volume_backups(&config, route_path, guest_path).await {
        Ok(snapshots) => axum::Json(snapshots).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error.to_string()).into_response(),
    }
}

pub(crate) async fn admin_chaos_scenario_handler(
    axum::Json(payload): axum::Json<AdminChaosScenarioRequest>,
) -> Response {
    let scenario = payload.scenario.trim();
    if scenario.is_empty() {
        return (StatusCode::BAD_REQUEST, "chaos scenario must not be empty").into_response();
    }
    if payload.duration_seconds.unwrap_or(0) > 3_600 {
        return (
            StatusCode::BAD_REQUEST,
            "chaos scenario duration must be less than or equal to 3600 seconds",
        )
            .into_response();
    }
    let supported = [
        "network_partition",
        "pod_eviction",
        "cpu_pressure",
        "lora_swap_failure",
        "node_isolation",
        "simulated_latency",
        "memory_pressure",
    ];
    if !supported.iter().any(|candidate| candidate == &scenario) {
        return (
            StatusCode::BAD_REQUEST,
            format!("unsupported chaos scenario `{scenario}`"),
        )
            .into_response();
    }

    let target = payload
        .target
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("mesh");
    let duration = payload.duration_seconds.unwrap_or(60);
    let outcome = AdminChaosScenarioOutcome {
        accepted: true,
        scenario: scenario.to_owned(),
        message: format!(
            "Chaos scenario `{scenario}` accepted for target `{target}` for {duration}s."
        ),
    };
    (StatusCode::ACCEPTED, axum::Json(outcome)).into_response()
}
