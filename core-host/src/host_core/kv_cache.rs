use crate::*;
use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use bytes::Bytes;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Return the tenant identifier from request headers.
/// Uses `x-tachyon-tenant` then `x-tenant-id`; falls back to `"default"`.
fn tenant_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("x-tachyon-tenant")
        .or_else(|| headers.get("x-tenant-id"))
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("default")
        .to_owned()
}

/// Return `true` when the component is currently loaded on this node.
/// Without the `ai-inference` feature the check always returns `false`
/// (no component can be hot), so writes are rejected in that build.
fn component_is_hot(state: &AppState, component_ref: &str) -> bool {
    #[cfg(feature = "ai-inference")]
    {
        state
            .runtime
            .load()
            .ai_runtime
            .loaded_component_aliases()
            .iter()
            .any(|alias| alias == component_ref)
    }
    #[cfg(not(feature = "ai-inference"))]
    {
        let _ = (state, component_ref);
        false
    }
}

/// Locate the `IntegrityKvCacheConfig` for `component_ref` in the running config.
fn cache_config_for<'a>(
    config: &'a IntegrityConfig,
    component_ref: &str,
) -> Option<&'a IntegrityKvCacheConfig> {
    config
        .kv_caches
        .iter()
        .find(|c| c.component_ref == component_ref)
}

// ── HTTP handlers ─────────────────────────────────────────────────────────────

/// `GET /api/component-cache/{component}/{key}`
///
/// Returns the cached bytes for the key, or 404 if absent / expired.
/// 404 is also returned when no `kv_caches` entry declares this component —
/// the caller should not cache for an undeclared component.
pub(crate) async fn kv_cache_get_handler(
    State(state): State<AppState>,
    Path((component_ref, cache_key)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let runtime = state.runtime.load();
    if cache_config_for(&runtime.config, &component_ref).is_none() {
        return (
            StatusCode::NOT_FOUND,
            format!("no component-cache configured for Component `{component_ref}`"),
        )
            .into_response();
    }
    let tenant = tenant_from_headers(&headers);
    match state
        .core_store
        .kv_cache_get(&component_ref, &tenant, &cache_key)
    {
        Ok(Some(value)) => (StatusCode::OK, Bytes::from(value)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("kv_cache read failed: {error:#}"),
        )
            .into_response(),
    }
}

/// `PUT /api/component-cache/{component}/{key}`
///
/// Stores `body` as the cache value for the key.
///
/// **Component-hot guard**: the write is refused with `503 Service Unavailable`
/// when the referenced LLM is not currently loaded on this node. This is the
/// core invariant that prevents cross-node cache pollution — a node that
/// doesn't run a component should never accumulate stale inference state for it.
pub(crate) async fn kv_cache_put_handler(
    State(state): State<AppState>,
    Path((component_ref, cache_key)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let runtime = state.runtime.load();
    let cache_cfg = match cache_config_for(&runtime.config, &component_ref) {
        Some(cfg) => cfg.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                format!("no component-cache configured for Component `{component_ref}`"),
            )
                .into_response();
        }
    };
    drop(runtime);

    // Core guard: refuse writes when the component is not hot on this node.
    if !component_is_hot(&state, &component_ref) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "Component `{component_ref}` is not loaded on this node — \
                 kv-cache writes are only accepted on the node that hosts the Component"
            ),
        )
            .into_response();
    }

    let tenant = if cache_cfg.tenant_isolation {
        tenant_from_headers(&headers)
    } else {
        "shared".to_owned()
    };

    match state.core_store.kv_cache_put(
        &component_ref,
        &tenant,
        &cache_key,
        &body,
        cache_cfg.max_ttl_seconds,
    ) {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("kv_cache write failed: {error:#}"),
        )
            .into_response(),
    }
}

/// `DELETE /api/component-cache/{component}/{key}`
///
/// Deletes a single cache entry.
pub(crate) async fn kv_cache_delete_handler(
    State(state): State<AppState>,
    Path((component_ref, cache_key)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let runtime = state.runtime.load();
    let cache_cfg = match cache_config_for(&runtime.config, &component_ref) {
        Some(cfg) => cfg.clone(),
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    drop(runtime);
    let tenant = if cache_cfg.tenant_isolation {
        tenant_from_headers(&headers)
    } else {
        "shared".to_owned()
    };
    match state
        .core_store
        .kv_cache_delete(&component_ref, &tenant, &cache_key)
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("kv_cache delete failed: {error:#}"),
        )
            .into_response(),
    }
}

/// `DELETE /admin/component-cache/{component}` — admin eviction of all entries for a Component.
///
/// Useful when a component is unloaded and its cached inference state is no longer
/// relevant. Returns `{ "evicted": N }`.
#[cfg_attr(not(feature = "admin-plane"), allow(dead_code))]
pub(crate) async fn kv_cache_evict_handler(
    State(state): State<AppState>,
    Path(component_ref): Path<String>,
) -> Response {
    match state.core_store.kv_cache_evict_component(&component_ref) {
        Ok(evicted) => (
            StatusCode::OK,
            axum::Json(serde_json::json!({ "component": component_ref, "evicted": evicted })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("kv_cache eviction failed: {error:#}"),
        )
            .into_response(),
    }
}

/// `GET /admin/component-cache/{component}/stats` — entry count and byte usage for a Component.
#[cfg_attr(not(feature = "admin-plane"), allow(dead_code))]
pub(crate) async fn kv_cache_stats_handler(
    State(state): State<AppState>,
    Path(component_ref): Path<String>,
) -> Response {
    match state.core_store.kv_cache_stats(&component_ref) {
        Ok(stats) => (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "component": component_ref,
                "entry_count": stats.entry_count,
                "total_bytes": stats.total_bytes,
                "expired_count": stats.expired_count,
            })),
        )
            .into_response(),
        Err(error) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("kv_cache stats failed: {error:#}"),
        )
            .into_response(),
    }
}
