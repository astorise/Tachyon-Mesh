use utoipa::OpenApi;

use crate::host_core::AdminRuntimeMetrics;

// ── Schema types ──────────────────────────────────────────────────────────────

#[derive(serde::Serialize, utoipa::ToSchema)]
pub(crate) struct RouteEntry {
    pub path: String,
    pub version: String,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestResponse {
    pub config_version: u64,
    pub routes: Vec<RouteEntry>,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestUpdateResult {
    pub success: bool,
    pub message: String,
    pub config_version: u64,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UserSummary {
    pub username: String,
    pub roles: Vec<String>,
    pub scopes: Vec<String>,
    pub groups: Vec<String>,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GroupSummary {
    pub name: String,
    pub roles: Vec<String>,
    pub scopes: Vec<String>,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CanaryStatusEntry {
    pub route_path: String,
    pub current_version: String,
    pub next_version: String,
    pub weight_pct: u32,
    pub phase: String,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShadowDiff {
    pub route: String,
    pub request_id: String,
    pub divergence: String,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AuditLogEntry {
    pub timestamp: String,
    pub actor: String,
    pub action: String,
    pub target: Option<String>,
    pub outcome: String,
    pub detail: Option<String>,
}

// ── ApiDoc ────────────────────────────────────────────────────────────────────

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Tachyon Mesh Admin API",
        description = "Administrative HTTP API exposed by the core-host runtime. All `/admin/*` routes require a valid bearer token.",
        version = "1.0.0"
    ),
    paths(
        // Status & metrics
        get_admin_status,
        get_admin_metrics,
        // Manifest
        get_admin_manifest,
        post_admin_manifest,
        post_admin_manifest_bundle,
        // KV-Partition V2
        get_admin_kv,
        put_admin_kv,
        delete_admin_kv,
        // Canary
        get_admin_canary,
        patch_admin_canary,
        post_admin_canary_abort,
        // Shadow traffic
        get_admin_shadow_diffs,
        // Chaos
        post_admin_chaos_scenario,
        // Identity
        get_admin_identity_public_key,
        // Enrollment
        post_admin_enrollment_start,
        post_admin_enrollment_approve,
        get_admin_enrollment_poll,
        // Security
        post_admin_security_recovery_codes,
        post_admin_security_2fa_regenerate,
        post_admin_security_step_up,
        post_admin_security_pats,
        // IAM
        get_admin_iam_users,
        patch_admin_iam_user,
        delete_admin_iam_user,
        get_admin_iam_groups,
        post_admin_iam_group,
        delete_admin_iam_group,
        get_admin_logs,
        // KV-Cache
        get_admin_kv_cache_stats,
        delete_admin_kv_cache,
        // Assets / models
        post_admin_assets,
        post_admin_models_init,
        put_admin_models_upload,
        post_admin_models_commit,
        // Schema & docs
        get_admin_schema_manifest,
        get_admin_schema_openapi,
        get_admin_schema_integrity_lock,
    ),
    components(schemas(
        AdminRuntimeMetrics,
        ManifestResponse,
        ManifestUpdateResult,
        RouteEntry,
        UserSummary,
        GroupSummary,
        CanaryStatusEntry,
        ShadowDiff,
        AuditLogEntry,
    )),
    tags(
        (name = "status",     description = "Runtime health and metrics"),
        (name = "manifest",   description = "Sealed configuration manifest operations"),
        (name = "kv",         description = "KV-Partition V2 distributed key-value store"),
        (name = "canary",     description = "Canary rollout traffic management"),
        (name = "traffic",    description = "Shadow traffic and routing"),
        (name = "chaos",      description = "Chaos engineering scenarios"),
        (name = "identity",   description = "Node identity and enrollment"),
        (name = "security",   description = "Operator security actions (MFA, PATs, step-up)"),
        (name = "iam",        description = "Identity and access management"),
        (name = "assets",     description = "Asset and model upload"),
        (name = "schema",     description = "Schema and documentation endpoints"),
    )
)]
pub(crate) struct ApiDoc;

pub(crate) fn get_base_openapi_schema() -> String {
    ApiDoc::openapi()
        .to_pretty_json()
        .unwrap_or_else(|_| "{}".to_owned())
}

// ── Path declarations ─────────────────────────────────────────────────────────
// These stub functions carry `#[utoipa::path]` metadata that `#[derive(OpenApi)]`
// on `ApiDoc` references via `paths(...)`. The macro expansion is invisible to
// the dead-code lint, so each stub carries `#[allow(dead_code)]` — this is a
// documented tool-chain exception, NOT an experimental-feature placebo.

#[utoipa::path(get, path = "/admin/status", tag = "status",
    security(("bearer_token" = [])),
    responses((status = 200, description = "Node is healthy"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn get_admin_status() {}

#[utoipa::path(get, path = "/admin/metrics", tag = "status",
    security(("bearer_token" = [])),
    responses((status = 200, description = "Runtime metrics", body = AdminRuntimeMetrics), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn get_admin_metrics() {}

#[utoipa::path(get, path = "/admin/manifest", tag = "manifest",
    security(("bearer_token" = [])),
    responses((status = 200, description = "Active sealed manifest", body = ManifestResponse), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn get_admin_manifest() {}

#[utoipa::path(post, path = "/admin/manifest", tag = "manifest",
    security(("bearer_token" = [])),
    request_body(content = serde_json::Value, description = "IntegrityConfig payload"),
    responses((status = 200, description = "Manifest applied", body = ManifestUpdateResult), (status = 400, description = "Validation error"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn post_admin_manifest() {}

#[utoipa::path(post, path = "/admin/manifest/bundle", tag = "manifest",
    security(("bearer_token" = [])),
    request_body(content = serde_json::Value, description = "Bundle request"),
    responses((status = 200, description = "Bundle applied"), (status = 409, description = "Version conflicts"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn post_admin_manifest_bundle() {}

// KV-Partition V2

#[utoipa::path(get, path = "/admin/kv/{namespace}/{key}", tag = "kv",
    params(("namespace" = String, Path, description = "Partition namespace"), ("key" = String, Path, description = "Entry key")),
    security(("bearer_token" = [])),
    responses((status = 200, description = "Value bytes"), (status = 404, description = "Key not found"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn get_admin_kv() {}

#[utoipa::path(put, path = "/admin/kv/{namespace}/{key}", tag = "kv",
    params(("namespace" = String, Path, description = "Partition namespace"), ("key" = String, Path, description = "Entry key")),
    security(("bearer_token" = [])),
    request_body(content = String, description = "Raw bytes (UTF-8 or base64)"),
    responses((status = 204, description = "Stored"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn put_admin_kv() {}

#[utoipa::path(delete, path = "/admin/kv/{namespace}/{key}", tag = "kv",
    params(("namespace" = String, Path, description = "Partition namespace"), ("key" = String, Path, description = "Entry key")),
    security(("bearer_token" = [])),
    responses((status = 204, description = "Deleted"), (status = 404, description = "Key not found"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn delete_admin_kv() {}

// Canary

#[utoipa::path(get, path = "/admin/canary", tag = "canary",
    security(("bearer_token" = [])),
    responses((status = 200, description = "Active canary rollouts", body = Vec<CanaryStatusEntry>), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn get_admin_canary() {}

#[utoipa::path(patch, path = "/admin/canary", tag = "canary",
    security(("bearer_token" = [])),
    request_body(content = serde_json::Value, description = "{ routePath, weightPct }"),
    responses((status = 200, description = "Weight updated"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn patch_admin_canary() {}

#[utoipa::path(post, path = "/admin/canary", tag = "canary",
    security(("bearer_token" = [])),
    request_body(content = serde_json::Value, description = "{ routePath } — aborts the rollout"),
    responses((status = 200, description = "Rollout aborted"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn post_admin_canary_abort() {}

// Shadow & chaos

#[utoipa::path(get, path = "/admin/shadow/diffs", tag = "traffic",
    security(("bearer_token" = [])),
    responses((status = 200, description = "Shadow traffic divergence records", body = Vec<ShadowDiff>), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn get_admin_shadow_diffs() {}

#[utoipa::path(post, path = "/admin/chaos/scenarios", tag = "chaos",
    security(("bearer_token" = [])),
    request_body(content = serde_json::Value, description = "{ scenario, duration_seconds?, target? }"),
    responses((status = 200, description = "Scenario accepted"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn post_admin_chaos_scenario() {}

// Identity & enrollment

#[utoipa::path(get, path = "/admin/identity/public-key", tag = "identity",
    security(("bearer_token" = [])),
    responses((status = 200, description = "Ed25519 public key (hex)"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn get_admin_identity_public_key() {}

#[utoipa::path(post, path = "/admin/enrollment/start", tag = "identity",
    security(("bearer_token" = [])),
    request_body(content = serde_json::Value, description = "Enrollment start request"),
    responses((status = 200, description = "Session created"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn post_admin_enrollment_start() {}

#[utoipa::path(post, path = "/admin/enrollment/approve", tag = "identity",
    security(("bearer_token" = [])),
    request_body(content = serde_json::Value, description = "Enrollment approval"),
    responses((status = 200, description = "Enrollment approved"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn post_admin_enrollment_approve() {}

#[utoipa::path(get, path = "/admin/enrollment/poll/{session_id}", tag = "identity",
    params(("session_id" = String, Path, description = "Enrollment session identifier")),
    security(("bearer_token" = [])),
    responses((status = 200, description = "Enrollment status"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn get_admin_enrollment_poll() {}

// Security

#[utoipa::path(post, path = "/admin/security/recovery-codes", tag = "security",
    security(("bearer_token" = [])),
    responses((status = 200, description = "Recovery codes generated"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn post_admin_security_recovery_codes() {}

#[utoipa::path(post, path = "/admin/security/2fa/regenerate", tag = "security",
    security(("bearer_token" = [])),
    responses((status = 200, description = "2FA secret regenerated"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn post_admin_security_2fa_regenerate() {}

#[utoipa::path(post, path = "/admin/security/step-up", tag = "security",
    security(("bearer_token" = [])),
    request_body(content = serde_json::Value, description = "Step-up session request"),
    responses((status = 200, description = "Step-up session issued"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn post_admin_security_step_up() {}

#[utoipa::path(post, path = "/admin/security/pats", tag = "security",
    security(("bearer_token" = [])),
    request_body(content = serde_json::Value, description = "PAT creation request"),
    responses((status = 200, description = "PAT issued"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn post_admin_security_pats() {}

// IAM

#[utoipa::path(get, path = "/admin/iam/users", tag = "iam",
    security(("bearer_token" = [])),
    responses((status = 200, description = "User summaries", body = Vec<UserSummary>), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn get_admin_iam_users() {}

#[utoipa::path(patch, path = "/admin/iam/users/{username}", tag = "iam",
    params(("username" = String, Path, description = "Target username")),
    security(("bearer_token" = [])),
    request_body(content = serde_json::Value, description = "UpdateUserRequest"),
    responses((status = 200, description = "Updated user", body = UserSummary), (status = 401, description = "Unauthorized"), (status = 404, description = "User not found")))]
#[allow(dead_code)]
fn patch_admin_iam_user() {}

#[utoipa::path(delete, path = "/admin/iam/users/{username}", tag = "iam",
    params(("username" = String, Path, description = "Username to delete")),
    security(("bearer_token" = [])),
    responses((status = 204, description = "User deleted"), (status = 401, description = "Unauthorized"), (status = 404, description = "User not found")))]
#[allow(dead_code)]
fn delete_admin_iam_user() {}

#[utoipa::path(get, path = "/admin/iam/groups", tag = "iam",
    security(("bearer_token" = [])),
    responses((status = 200, description = "Group summaries", body = Vec<GroupSummary>), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn get_admin_iam_groups() {}

#[utoipa::path(post, path = "/admin/iam/groups", tag = "iam",
    security(("bearer_token" = [])),
    request_body(content = serde_json::Value, description = "UpsertGroupRequest"),
    responses((status = 200, description = "Group upserted", body = GroupSummary), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn post_admin_iam_group() {}

#[utoipa::path(delete, path = "/admin/iam/groups/{name}", tag = "iam",
    params(("name" = String, Path, description = "Group name")),
    security(("bearer_token" = [])),
    responses((status = 204, description = "Group deleted"), (status = 401, description = "Unauthorized"), (status = 404, description = "Group not found")))]
#[allow(dead_code)]
fn delete_admin_iam_group() {}

#[utoipa::path(get, path = "/admin/logs", tag = "iam",
    security(("bearer_token" = [])),
    responses((status = 200, description = "IAM audit log entries", body = Vec<AuditLogEntry>), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn get_admin_logs() {}

// KV-Cache

#[utoipa::path(get, path = "/admin/kv-cache/{model}/stats", tag = "status",
    params(("model" = String, Path, description = "Model reference identifier")),
    security(("bearer_token" = [])),
    responses((status = 200, description = "KV-cache statistics"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn get_admin_kv_cache_stats() {}

#[utoipa::path(delete, path = "/admin/kv-cache/{model}", tag = "status",
    params(("model" = String, Path, description = "Model reference identifier")),
    security(("bearer_token" = [])),
    responses((status = 204, description = "KV-cache evicted"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn delete_admin_kv_cache() {}

// Assets & models

#[utoipa::path(post, path = "/admin/assets", tag = "assets",
    security(("bearer_token" = [])),
    request_body(content = String, description = "Raw asset bytes"),
    responses((status = 200, description = "Asset stored, returns asset path"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn post_admin_assets() {}

#[utoipa::path(post, path = "/admin/models/init", tag = "assets",
    security(("bearer_token" = [])),
    request_body(content = serde_json::Value, description = "Upload init request { name, size_bytes }"),
    responses((status = 200, description = "Upload session created, returns upload_id"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn post_admin_models_init() {}

#[utoipa::path(put, path = "/admin/models/upload/{upload_id}", tag = "assets",
    params(("upload_id" = String, Path, description = "Upload session identifier")),
    security(("bearer_token" = [])),
    request_body(content = String, description = "Chunk bytes"),
    responses((status = 204, description = "Chunk received"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn put_admin_models_upload() {}

#[utoipa::path(post, path = "/admin/models/commit/{upload_id}", tag = "assets",
    params(("upload_id" = String, Path, description = "Upload session identifier")),
    security(("bearer_token" = [])),
    responses((status = 200, description = "Model committed"), (status = 401, description = "Unauthorized")))]
#[allow(dead_code)]
fn post_admin_models_commit() {}

// Schema & docs

#[utoipa::path(get, path = "/admin/schema/manifest", tag = "schema",
    security(("bearer_token" = [])),
    responses((status = 200, description = "JSON Schema (Draft-07) for IntegrityConfig")))]
#[allow(dead_code)]
fn get_admin_schema_manifest() {}

#[utoipa::path(get, path = "/admin/schema/openapi.json", tag = "schema",
    security(("bearer_token" = [])),
    responses((status = 200, description = "OpenAPI 3.1 JSON document")))]
#[allow(dead_code)]
fn get_admin_schema_openapi() {}

#[utoipa::path(get, path = "/admin/schema/integrity-lock", tag = "schema",
    security(("bearer_token" = [])),
    responses((status = 200, description = "JSON Schema for the integrity.lock file format")))]
#[allow(dead_code)]
fn get_admin_schema_integrity_lock() {}
