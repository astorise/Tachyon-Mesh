use utoipa::OpenApi;

use crate::host_core::AdminRuntimeMetrics;

/// Serializable summary of a sealed manifest route entry.
#[derive(serde::Serialize, utoipa::ToSchema)]
pub(crate) struct RouteEntry {
    pub path: String,
    pub version: String,
}

/// Response from GET /admin/manifest.
#[derive(serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestResponse {
    pub config_version: u64,
    pub routes: Vec<RouteEntry>,
}

/// Success response from POST /admin/manifest.
#[derive(serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManifestUpdateResult {
    pub success: bool,
    pub message: String,
    pub config_version: u64,
}

/// User summary returned by IAM list endpoints.
#[derive(serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UserSummary {
    pub username: String,
    pub roles: Vec<String>,
    pub scopes: Vec<String>,
    pub groups: Vec<String>,
}

/// Group summary returned by IAM list endpoints.
#[derive(serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GroupSummary {
    pub name: String,
    pub roles: Vec<String>,
    pub scopes: Vec<String>,
}

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Tachyon Mesh Admin API",
        description = "Administrative HTTP API exposed by the core-host runtime. All `/admin/*` routes require a valid bearer token.",
        version = "1.0.0"
    ),
    paths(
        get_admin_status,
        get_admin_metrics,
        get_admin_manifest,
        post_admin_manifest,
        post_admin_manifest_bundle,
        get_admin_schema_manifest,
        get_admin_schema_openapi,
        get_admin_iam_users,
        patch_admin_iam_user,
        get_admin_iam_groups,
    ),
    components(schemas(
        AdminRuntimeMetrics,
        ManifestResponse,
        ManifestUpdateResult,
        RouteEntry,
        UserSummary,
        GroupSummary,
    )),
    tags(
        (name = "status", description = "Runtime health and metrics"),
        (name = "manifest", description = "Sealed configuration manifest operations"),
        (name = "iam", description = "Identity and access management"),
        (name = "schema", description = "Schema and documentation endpoints"),
    )
)]
pub(crate) struct ApiDoc;

/// Returns the base OpenAPI schema as a pretty-printed JSON string.
pub(crate) fn get_base_openapi_schema() -> String {
    ApiDoc::openapi()
        .to_pretty_json()
        .unwrap_or_else(|_| "{}".to_owned())
}

// --- Path declarations (shadow the actual handlers for utoipa metadata only) ---

/// Get node admin status and authentication info.
#[utoipa::path(
    get,
    path = "/admin/status",
    tag = "status",
    security(("bearer_token" = [])),
    responses(
        (status = 200, description = "Node is healthy"),
        (status = 401, description = "Unauthorized"),
    )
)]
#[allow(dead_code)]
fn get_admin_status() {}

/// Get runtime telemetry metrics snapshot.
#[utoipa::path(
    get,
    path = "/admin/metrics",
    tag = "status",
    security(("bearer_token" = [])),
    responses(
        (status = 200, description = "Runtime metrics", body = AdminRuntimeMetrics),
        (status = 401, description = "Unauthorized"),
    )
)]
#[allow(dead_code)]
fn get_admin_metrics() {}

/// Get the currently sealed manifest configuration.
#[utoipa::path(
    get,
    path = "/admin/manifest",
    tag = "manifest",
    security(("bearer_token" = [])),
    responses(
        (status = 200, description = "Sealed manifest", body = ManifestResponse),
        (status = 401, description = "Unauthorized"),
    )
)]
#[allow(dead_code)]
fn get_admin_manifest() {}

/// Apply a new manifest configuration.
#[utoipa::path(
    post,
    path = "/admin/manifest",
    tag = "manifest",
    security(("bearer_token" = [])),
    request_body(content = serde_json::Value, description = "IntegrityConfig payload"),
    responses(
        (status = 200, description = "Manifest applied", body = ManifestUpdateResult),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Unauthorized"),
    )
)]
#[allow(dead_code)]
fn post_admin_manifest() {}

/// Bundle dependencies and apply a manifest in one operation.
#[utoipa::path(
    post,
    path = "/admin/manifest/bundle",
    tag = "manifest",
    security(("bearer_token" = [])),
    request_body(content = serde_json::Value, description = "Bundle request with dependency list"),
    responses(
        (status = 200, description = "Bundle applied"),
        (status = 409, description = "Version conflicts require resolution"),
        (status = 401, description = "Unauthorized"),
    )
)]
#[allow(dead_code)]
fn post_admin_manifest_bundle() {}

/// Get JSON Schema for the IntegrityConfig manifest format.
#[utoipa::path(
    get,
    path = "/admin/schema/manifest",
    tag = "schema",
    security(("bearer_token" = [])),
    responses(
        (status = 200, description = "JSON Schema (Draft-07) for IntegrityConfig", content_type = "application/json"),
        (status = 401, description = "Unauthorized"),
    )
)]
#[allow(dead_code)]
fn get_admin_schema_manifest() {}

/// Get OpenAPI 3.1 schema for the Tachyon Mesh Admin API.
#[utoipa::path(
    get,
    path = "/admin/schema/openapi.json",
    tag = "schema",
    security(("bearer_token" = [])),
    responses(
        (status = 200, description = "OpenAPI 3.1 JSON document", content_type = "application/json"),
        (status = 401, description = "Unauthorized"),
    )
)]
#[allow(dead_code)]
fn get_admin_schema_openapi() {}

/// List all enrolled users.
#[utoipa::path(
    get,
    path = "/admin/iam/users",
    tag = "iam",
    security(("bearer_token" = [])),
    responses(
        (status = 200, description = "User summaries", body = Vec<UserSummary>),
        (status = 401, description = "Unauthorized"),
    )
)]
#[allow(dead_code)]
fn get_admin_iam_users() {}

/// Update a user's groups, roles, scopes, or disabled state.
#[utoipa::path(
    patch,
    path = "/admin/iam/users/{username}",
    tag = "iam",
    params(("username" = String, Path, description = "Target username")),
    security(("bearer_token" = [])),
    request_body(content = serde_json::Value, description = "UpdateUserRequest"),
    responses(
        (status = 200, description = "Updated user summary", body = UserSummary),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "User not found"),
    )
)]
#[allow(dead_code)]
fn patch_admin_iam_user() {}

/// List all groups.
#[utoipa::path(
    get,
    path = "/admin/iam/groups",
    tag = "iam",
    security(("bearer_token" = [])),
    responses(
        (status = 200, description = "Group summaries", body = Vec<GroupSummary>),
        (status = 401, description = "Unauthorized"),
    )
)]
#[allow(dead_code)]
fn get_admin_iam_groups() {}
