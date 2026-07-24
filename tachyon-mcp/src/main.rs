#![recursion_limit = "256"]

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    env,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::warn;

/// Set once per process on the first authenticated request; subsequent requests
/// skip the `set_connection` HTTP round-trip and reuse the cached state held
/// inside `tachyon_client`'s global connection registry.
static CONNECTION_INITIALIZED: OnceLock<()> = OnceLock::new();

/// Cached JSON Schema for the `IntegrityConfig` manifest, fetched once from
/// `GET /admin/schema/manifest` immediately after the connection is established.
/// Injected into the `tachyon_dryrun_manifest` tool definition so that agents
/// receive precise field-level guidance directly in the tool schema.
static MANIFEST_SCHEMA: OnceLock<Value> = OnceLock::new();

const RATE_LIMIT_WINDOW_SECS: u64 = 60;
const RATE_LIMIT_PERSIST_SECS: u64 = 10;
const DEFAULT_TIMEOUT_MS: u64 = 5_000;

/// Returns the per-request timeout for tachyon_client calls from the optional
/// `TACHYON_MCP_TIMEOUT_MS` environment variable, falling back to 5 000 ms.
fn mcp_timeout() -> Duration {
    let ms = env::var("TACHYON_MCP_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_TIMEOUT_MS);
    Duration::from_millis(ms)
}

// ── Structured JSON-RPC error taxonomy ───────────────────────────────────────

/// Structured JSON-RPC 2.0 error object.
/// Serialises to `{ "code": …, "message": "…", "data": … }` and is embedded
/// inside the top-level `error` field of a JSON-RPC error response.
#[derive(Debug, Serialize)]
pub(crate) struct JsonRpcError {
    pub(crate) code: i32,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) data: Option<Value>,
}

impl JsonRpcError {
    /// `-32602 Invalid params` — caller supplied a payload that fails
    /// structural or semantic validation.
    pub(crate) fn invalid_params(msg: &str, details: Value) -> Self {
        Self {
            code: -32602,
            message: msg.to_owned(),
            data: Some(details),
        }
    }

    /// `-32001 Cluster unreachable` — the tachyon_client call timed out or
    /// could not reach core-host.
    pub(crate) fn cluster_unreachable(msg: &str) -> Self {
        Self {
            code: -32001,
            message: msg.to_owned(),
            data: None,
        }
    }

    /// `-32002 Rate limited` — includes `retry_after_ms` in the `data` field.
    pub(crate) fn rate_limited(retry_after_ms: u64) -> Self {
        Self {
            code: -32002,
            message: "Rate limit exceeded. Retry after the indicated delay.".to_owned(),
            data: Some(json!({ "retry_after_ms": retry_after_ms })),
        }
    }

    /// `-32603 Internal error` — unexpected failure with a human-readable
    /// message forwarded from the underlying error.
    pub(crate) fn internal_error(msg: &str) -> Self {
        Self {
            code: -32603,
            message: msg.to_owned(),
            data: None,
        }
    }

    /// Classify an `anyhow::Error` into the appropriate `JsonRpcError` variant.
    /// Checks the error chain for well-known signal strings produced by
    /// `tokio::time::timeout` and tachyon_client network failures.
    pub(crate) fn from_anyhow(error: &anyhow::Error) -> Self {
        let msg = error.to_string();
        if msg.contains("__TIMEOUT__") || msg.contains("timed out") || msg.contains("deadline") {
            Self::cluster_unreachable(&format!(
                "core-host did not respond within {}ms",
                DEFAULT_TIMEOUT_MS
            ))
        } else if msg.contains("connection refused")
            || msg.contains("failed to connect")
            || msg.contains("unreachable")
        {
            Self::cluster_unreachable(&msg)
        } else if msg.contains("validation") || msg.contains("invalid") || msg.contains("schema") {
            Self::invalid_params("Manifest validation failed", json!({ "detail": msg }))
        } else if msg.starts_with("missing ") || msg.contains(": missing ") {
            // `.context("missing X")?` propagation: caller omitted a required field.
            Self::invalid_params("Missing required field", json!({ "detail": msg }))
        } else {
            Self::internal_error(&msg)
        }
    }
}

/// Build a complete JSON-RPC 2.0 error response from a [`JsonRpcError`].
fn json_rpc_error_response(id: Option<Value>, err: &JsonRpcError) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": err,
    })
}

struct McpContext {
    token: String,
    url: String,
    rate_limiter: ToolRateLimiter,
    enforce_remote_auth: bool,
}

#[derive(Clone, Copy)]
struct RateLimitSpec {
    limit: u32,
    window_secs: u64,
}

struct TokenBucket {
    limit: u32,
    tokens: u32,
    last_refill_unix: u64,
}

impl TokenBucket {
    fn new(spec: RateLimitSpec, now: u64) -> Self {
        Self {
            limit: spec.limit,
            tokens: spec.limit,
            last_refill_unix: now,
        }
    }

    /// Returns `None` when the call is permitted, or `Some(retry_after_ms)`
    /// when the bucket is exhausted. `retry_after_ms` is the number of
    /// milliseconds until the current window resets.
    fn allow(&mut self, spec: RateLimitSpec, now: u64) -> Option<u64> {
        self.limit = spec.limit;
        if now.saturating_sub(self.last_refill_unix) >= spec.window_secs {
            self.tokens = spec.limit;
            self.last_refill_unix = now;
        }
        if self.tokens == 0 {
            let reset_at = self.last_refill_unix + spec.window_secs;
            let retry_after_ms = reset_at.saturating_sub(now).saturating_mul(1_000);
            return Some(retry_after_ms);
        }
        self.tokens -= 1;
        None
    }
}

struct ToolRateLimiter {
    state_path: PathBuf,
    state: Mutex<ToolRateLimiterState>,
}

struct ToolRateLimiterState {
    buckets: HashMap<String, TokenBucket>,
    last_persist_unix: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct PersistedRateLimitState {
    buckets: HashMap<String, PersistedTokenBucket>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedTokenBucket {
    limit: u32,
    tokens: u32,
    last_refill_unix: u64,
}

impl ToolRateLimiter {
    fn new() -> Self {
        let state_path = env::var_os("TACHYON_MCP_RATE_LIMIT_STATE")
            .map(PathBuf::from)
            .unwrap_or_else(|| env::temp_dir().join("tachyon-mcp-rate-limits.state"));
        Self::new_with_path(state_path)
    }

    fn new_with_path(state_path: PathBuf) -> Self {
        let now = unix_now();
        let buckets = load_rate_limit_state(&state_path)
            .unwrap_or_default()
            .buckets
            .into_iter()
            .map(|(name, bucket)| {
                (
                    name,
                    TokenBucket {
                        limit: bucket.limit,
                        tokens: bucket.tokens,
                        last_refill_unix: bucket.last_refill_unix,
                    },
                )
            })
            .collect();
        Self {
            state_path,
            state: Mutex::new(ToolRateLimiterState {
                buckets,
                last_persist_unix: now,
            }),
        }
    }

    /// Returns `None` when the call is permitted, or `Some(retry_after_ms)`
    /// when the tool's bucket is exhausted.
    fn allow(&self, tool_name: &str) -> Result<Option<u64>> {
        let Some(spec) = rate_limit_spec(tool_name) else {
            return Ok(None);
        };
        let now = unix_now();
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("MCP rate limiter lock is poisoned"))?;
        let denied_ms = state
            .buckets
            .entry(tool_name.to_owned())
            .or_insert_with(|| TokenBucket::new(spec, now))
            .allow(spec, now);
        let was_denied = denied_ms.is_some();
        if was_denied || now.saturating_sub(state.last_persist_unix) >= RATE_LIMIT_PERSIST_SECS {
            persist_rate_limit_state(&self.state_path, &state.buckets)?;
            state.last_persist_unix = now;
        }
        Ok(denied_ms)
    }
}

impl McpContext {
    fn new(token: String, url: String) -> Self {
        Self {
            token,
            url,
            rate_limiter: ToolRateLimiter::new(),
            enforce_remote_auth: true,
        }
    }

    #[cfg(test)]
    fn new_for_tests(state_path: PathBuf) -> Self {
        Self {
            token: "test-token".to_owned(),
            url: "http://127.0.0.1:1".to_owned(),
            rate_limiter: ToolRateLimiter::new_with_path(state_path),
            enforce_remote_auth: false,
        }
    }
}

/// Returns the names of any required arguments missing from the `arguments`
/// payload of a `tools/call` request. Mirrors the `required` arrays declared
/// in `tools/list` inputSchemas. Returning `None` means "all required fields
/// are present" (or that the tool has no required fields).
///
/// This check runs before auth/network so missing fields surface as
/// `-32602 invalid_params` rather than `-32001 cluster_unreachable`.
fn missing_required_args(tool_name: &str, arguments: Option<&Value>) -> Option<Vec<String>> {
    let required: &[&str] = match tool_name {
        "tachyon_import_package" => &["package_path"],
        "tachyon_upload_model" => &["path"],
        "tachyon_deploy_function" => &["function_name", "artifact_path"],
        "tachyon_delete_function" => &["function_name"],
        "tachyon_function_logs" => &["function_name"],
        "tachyon_kv_get" => &["namespace", "key"],
        "tachyon_kv_put" => &["namespace", "key", "value"],
        "tachyon_kv_delete" => &["namespace", "key"],
        "tachyon_kv_cache_stats" | "tachyon_kv_cache_flush" => &["model"],
        "tachyon_vector_search" => &["query", "index", "top_k"],
        "tachyon_canary_split" => &["route_path", "weight_pct"],
        "tachyon_register_resource" => &["name", "type", "target"],
        "tachyon_dryrun_manifest" => &["manifest"],
        "tachyon_run_chaos_scenario" => &["scenario"],
        "list_s3_volumes" => &["route_path"],
        "attach_s3_volume" => &["route_path", "s3_url", "guest_path"],
        "detach_s3_volume" => &["route_path", "guest_path"],
        "list_volume_backups" => &["route_path", "guest_path"],
        "backup_volume" => &["route_path", "guest_path"],
        "restore_volume" => &["route_path", "guest_path", "snapshot_id"],
        "recommend_concurrency_policy" => &["pattern"],
        "tachyon_set_route_scopes" => &["route_path", "scopes"],
        "tachyon_patch_manifest" => &["patch"],
        "tachyon_patch_route" => &["route_path", "patch"],
        "tachyon_lora_training_status" => &["job_id"],
        "tachyon_suggest_scopes" => &["route_path"],
        _ => return None,
    };
    let obj = match arguments.and_then(Value::as_object) {
        Some(map) => map,
        None => {
            // No arguments object at all → every required field is missing.
            return Some(required.iter().map(|s| (*s).to_owned()).collect());
        }
    };
    let missing: Vec<String> = required
        .iter()
        .filter(|field| !obj.contains_key(**field))
        .map(|s| (*s).to_owned())
        .collect();
    if missing.is_empty() {
        None
    } else {
        Some(missing)
    }
}

fn rate_limit_spec(tool_name: &str) -> Option<RateLimitSpec> {
    let limit = match tool_name {
        // Critical mutators — very tight budget to prevent accidental canary misconfiguration.
        "tachyon_canary_split" => 2,
        // Deployment / deletion mutators — moderate budget.
        "tachyon_apply_manifest"
        | "tachyon_seal_overlay"
        | "tachyon_set_route_scopes"
        | "tachyon_patch_manifest"
        | "tachyon_patch_route" => 1,
        "tachyon_import_package" => 3,
        // Model uploads are large and hash-verified — keep the budget tight.
        "tachyon_upload_model" => 3,
        "tachyon_deploy_function" | "tachyon_delete_function" => 5,
        "tachyon_register_resource" => 10,
        // KV mutators and log fetches — generous but bounded.
        "tachyon_kv_put"
        | "tachyon_kv_delete"
        | "tachyon_kv_cache_flush"
        | "tachyon_function_logs" => 30,
        // Read-only telemetry and scope tools.
        "tachyon_get_metrics"
        | "tachyon_tail_logs"
        | "tachyon_get_scope_denials"
        | "tachyon_suggest_scopes" => 30,
        // S3 volume mutators — moderate budget.
        "attach_s3_volume" | "detach_s3_volume" | "backup_volume" | "restore_volume" => 10,
        "list_volume_backups" => 60,
        // Recommendation is pure local computation, no I/O.
        "recommend_concurrency_policy" => 100,
        // All remaining read-only tools — high throughput allowed.
        "tachyon_mesh_status"
        | "tachyon_lockfile"
        | "tachyon_topology_snapshot"
        | "tachyon_hardware_status"
        | "tachyon_list_resources"
        | "tachyon_list_functions"
        | "tachyon_lora_training_status"
        | "tachyon_kv_get"
        | "tachyon_kv_cache_stats"
        | "tachyon_vector_search"
        | "tachyon_dryrun_manifest"
        | "validate_faas_capabilities"
        | "run_chaos_scenario"
        | "list_s3_volumes" => 100,
        _ => return None,
    };
    Some(RateLimitSpec {
        limit,
        window_secs: RATE_LIMIT_WINDOW_SECS,
    })
}

fn load_rate_limit_state(path: &PathBuf) -> Result<PersistedRateLimitState> {
    let raw = std::fs::read(path)
        .with_context(|| format!("failed to read MCP rate-limit state `{}`", path.display()))?;
    serde_json::from_slice(&raw)
        .with_context(|| format!("failed to decode MCP rate-limit state `{}`", path.display()))
}

fn persist_rate_limit_state(path: &PathBuf, buckets: &HashMap<String, TokenBucket>) -> Result<()> {
    let persisted = PersistedRateLimitState {
        buckets: buckets
            .iter()
            .map(|(name, bucket)| {
                (
                    name.clone(),
                    PersistedTokenBucket {
                        limit: bucket.limit,
                        tokens: bucket.tokens,
                        last_refill_unix: bucket.last_refill_unix,
                    },
                )
            })
            .collect(),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create MCP rate-limit state directory `{}`",
                parent.display()
            )
        })?;
    }
    let tmp = path.with_extension("state.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&persisted)?)
        .with_context(|| format!("failed to write MCP rate-limit state `{}`", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("failed to commit MCP rate-limit state `{}`", path.display()))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_secs()
}

#[tokio::main]
async fn main() -> Result<()> {
    let context = McpContext::new(load_required_token()?, load_required_url()?);

    let stdin = BufReader::new(io::stdin());
    let mut lines = stdin.lines();
    let mut stdout = io::stdout();

    while let Some(line) = lines.next_line().await.context("failed to read stdin")? {
        if line.trim().is_empty() {
            continue;
        }

        match handle_line(&line, &context).await {
            Ok(Some(response)) => {
                stdout
                    .write_all(response.to_string().as_bytes())
                    .await
                    .context("failed to write JSON-RPC response")?;
                stdout
                    .write_all(b"\n")
                    .await
                    .context("failed to terminate JSON-RPC response")?;
                stdout.flush().await.context("failed to flush stdout")?;
            }
            Ok(None) => {}
            Err(error) => {
                eprintln!("tachyon-mcp error: {error:#}");
                let response = json_rpc_error_response(None, &JsonRpcError::from_anyhow(&error));
                stdout
                    .write_all(response.to_string().as_bytes())
                    .await
                    .context("failed to write JSON-RPC error response")?;
                stdout
                    .write_all(b"\n")
                    .await
                    .context("failed to terminate JSON-RPC error response")?;
                stdout.flush().await.context("failed to flush stdout")?;
            }
        }
    }

    Ok(())
}

fn load_required_token() -> Result<String> {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--token" {
            let token = args.next().context("--token requires a PAT value")?;
            if !token.trim().is_empty() {
                return Ok(token);
            }
        }
        if let Some(token) = arg.strip_prefix("--token=") {
            if !token.trim().is_empty() {
                return Ok(token.to_owned());
            }
        }
    }

    let token = env::var("TACHYON_MCP_PAT").context(
        "tachyon-mcp requires a PAT via --token <pat> or TACHYON_MCP_PAT before accepting requests",
    )?;
    if token.trim().is_empty() {
        anyhow::bail!("tachyon-mcp PAT must not be empty");
    }
    Ok(token)
}

fn load_required_url() -> Result<String> {
    let url = env::var("TACHYON_MCP_URL")
        .context("tachyon-mcp requires TACHYON_MCP_URL before accepting requests")?;
    if url.trim().is_empty() {
        anyhow::bail!("TACHYON_MCP_URL must not be empty");
    }
    Ok(url)
}

async fn handle_line(line: &str, context: &McpContext) -> Result<Option<Value>> {
    let request: Value =
        serde_json::from_str(line).with_context(|| format!("invalid JSON-RPC payload: {line}"))?;
    let id = request.get("id").cloned();
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .context("missing JSON-RPC method")?;

    if id.is_none() {
        return Ok(None);
    }

    if method == "tools/call" {
        let tool_name = request
            .get("params")
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .context("missing tool name")?;
        if let Some(rate_err) = check_rate_limit(context, tool_name)? {
            return Ok(Some(json_rpc_error_response(id, &rate_err)));
        }
        // Validate required tool arguments BEFORE auth/network so malformed
        // requests surface a precise `-32602 invalid_params` regardless of
        // cluster reachability. Agents rely on this exact code to self-correct.
        let arguments = request
            .get("params")
            .and_then(|value| value.get("arguments"));
        if let Some(missing) = missing_required_args(tool_name, arguments) {
            let rpc_err = JsonRpcError::invalid_params(
                "Missing required field",
                json!({ "tool": tool_name, "missing": missing }),
            );
            return Ok(Some(json_rpc_error_response(id, &rpc_err)));
        }
    }

    if method != "initialize" {
        if let Err(error) = validate_request_auth(context).await {
            let rpc_err = JsonRpcError::cluster_unreachable(&error.to_string());
            return Ok(Some(json_rpc_error_response(id, &rpc_err)));
        }
    }

    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2025-03-26",
            "serverInfo": {
                "name": "tachyon-mcp",
                "version": env!("CARGO_PKG_VERSION")
            },
            "capabilities": {
                "tools": {},
                "resources": {}
            }
        }),
        "resources/list" => json!({
            "resources": [
                {
                    "uri": "hardware://local/status",
                    "name": "Local hardware status",
                    "description": "Current local RAM and accelerator availability for sizing Tachyon FaaS manifests.",
                    "mimeType": "application/json"
                },
                {
                    "uri": "hardware://mesh/cluster",
                    "name": "Mesh hardware summary",
                    "description": "Cluster-level enrolled-node, RAM, and GPU summary from the Tachyon node registry.",
                    "mimeType": "application/json"
                },
                {
                    "uri": "hardware://mesh/{node_id}/status",
                    "name": "Mesh node hardware status",
                    "description": "Per-node capabilities and GPU status from the Tachyon node registry. Replace {node_id} with an enrolled node id.",
                    "mimeType": "application/json"
                }
            ]
        }),
        "resources/read" => {
            let uri = request
                .get("params")
                .and_then(|value| value.get("uri"))
                .and_then(Value::as_str)
                .context("missing resource uri")?;
            let text = if uri == "hardware://local/status" {
                let status =
                    tokio::task::spawn_blocking(tachyon_client::read_local_hardware_status)
                        .await
                        .context("hardware status task panicked")?;
                serde_json::to_string_pretty(&status)?
            } else if uri == "hardware://mesh/cluster" {
                let summary = tachyon_client::get_cluster_hardware_summary().await?;
                serde_json::to_string_pretty(&summary)?
            } else if let Some(node_id) = uri
                .strip_prefix("hardware://mesh/")
                .and_then(|value| value.strip_suffix("/status"))
                .filter(|value| !value.trim().is_empty() && *value != "{node_id}")
            {
                let capabilities = tachyon_client::get_node_capabilities(node_id).await?;
                serde_json::to_string_pretty(&capabilities)?
            } else {
                return Ok(Some(json_rpc_error_response(
                    id,
                    &JsonRpcError::invalid_params("unsupported resource", json!({ "uri": uri })),
                )));
            };
            json!({
                "contents": [
                    {
                        "uri": uri,
                        "mimeType": "application/json",
                        "text": text
                    }
                ]
            })
        }
        "tools/list" => {
            let mut tools_result = json!({
            "tools": [
                {
                    "name": "tachyon_mesh_status",
                    "description": "Return the current summarized engine status from integrity.lock",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "tachyon_lockfile",
                    "description": "Return the current integrity.lock payload",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "tachyon_list_resources",
                    "description": "List logical mesh resources (sealed in integrity.lock plus pending overlay entries) so an AI can discover existing internal IPC aliases and external HTTPS egress targets.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "tachyon_register_resource",
                    "description": "Register a new mesh resource into the workspace overlay (tachyon.resources.json). The entry is persisted as `pending` and requires tachyon_seal_overlay plus tachyon_apply_manifest to take effect.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["name", "type", "target"],
                        "properties": {
                            "name": { "type": "string", "description": "Logical alias used inside the mesh, e.g. `stripe-api`." },
                            "type": { "type": "string", "enum": ["internal", "external"] },
                            "target": { "type": "string", "description": "For external: HTTPS URL (or http:// for loopback / *.svc cluster-local). For internal: IPC URI like `wasm://module`." },
                            "allowedMethods": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "External-only: list of allowed HTTP methods such as [\"GET\", \"POST\"]."
                            },
                            "versionConstraint": {
                                "type": "string",
                                "description": "Internal-only: semver constraint such as `^1.2.0`."
                            }
                        }
                    }
                },
                {
                    "name": "tachyon_seal_overlay",
                    "description": "Seal the local Tachyon overlay into integrity.lock using the local Ed25519 key and incremented config_version.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "tachyon_apply_manifest",
                    "description": "POST the currently sealed integrity.lock manifest to the active Tachyon host /admin/manifest endpoint.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "tachyon_dryrun_manifest",
                    "description": "Validate a Tachyon manifest payload without writing the overlay, integrity.lock, or remote node state.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["manifest"],
                        "properties": {
                            "manifest": MANIFEST_SCHEMA.get().cloned().unwrap_or_else(|| json!({
                                "type": "object",
                                "description": "IntegrityConfig payload. Connect to a running core-host to receive the full JSON Schema."
                            }))
                        }
                    }
                },
                {
                    "name": "tachyon_get_metrics",
                    "description": "Return active node telemetry: error rate, p50/p99 latency, queue depth, scope_denial_total, and mesh_dispatch counters/latency aggregates for in_process, uds, and tcp dispatch modes.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "tachyon_get_scope_denials",
                    "description": "Return the lifetime count of WIT import scope denials from the active node. Optionally filter by route_path to inspect the allow_all flag for a specific route. Per-category and per-deployment breakdowns are available via prometheus (faas_scope_denials_total{deployment,category}).",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "route_path": {
                                "type": "string",
                                "description": "Optional HTTP route path (e.g. '/api/my-fn'). When provided, response includes the allow_all flag derived from the manifest."
                            }
                        }
                    }
                },
                {
                    "name": "tachyon_set_route_scopes",
                    "description": "Apply a scopes block to a route in the live manifest. Reads the current manifest, merges the provided scopes into the target route, and POSTs the modified manifest. Use dry_run=true to preview the manifest change without applying it. Caution: concurrent manifest edits will overwrite each other.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["route_path", "scopes"],
                        "properties": {
                            "route_path": {
                                "type": "string",
                                "description": "HTTP route path of the target deployment (e.g. '/api/my-fn')."
                            },
                            "scopes": {
                                "type": "object",
                                "description": "Scopes block to merge. Keys are WIT categories (secrets, kv, graph, sql, http, blob, messaging, crypto, routing, compute); values are arrays of glob patterns or 'allow-all'."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "default": false,
                                "description": "When true, return manifest_preview without posting to the node."
                            }
                        }
                    }
                },
                {
                    "name": "tachyon_patch_route",
                    "description": "Patch any configurable fields on a route in the live manifest. Reads the current manifest, recursively merges the JSON patch object into the target route using JSON Merge Patch semantics: object fields merge recursively, null removes the target key, and missing keys are left unchanged. It validates through the node manifest endpoint when applying, and POSTs the modified manifest. Use dry_run=true to preview the merged route and manifest without applying it. The structural fields path and role cannot be patched, including via null.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["route_path", "patch"],
                        "properties": {
                            "route_path": {
                                "type": "string",
                                "description": "HTTP route path of the target deployment (e.g. '/api/my-fn')."
                            },
                            "patch": {
                                "type": "object",
                                "description": "Route fragment to merge, such as {\"concurrency\":{\"mode\":\"mesh-singleton\",\"on_conflict\":\"queue\"},\"distributed_rate_limit\":{\"threshold\":100,\"window_seconds\":60,\"scope\":\"tenant\"},\"adapter_id\":\"tenant-a\"}. Set a field to null to remove it, for example {\"canary\":null,\"shadow_target\":null}."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "default": false,
                                "description": "When true, return route_preview and manifest_preview without posting to the node."
                            }
                        }
                    }
                },
                {
                    "name": "tachyon_patch_manifest",
                    "description": "Patch configurable host-level fields in the live manifest. Reads the current manifest, recursively merges the JSON patch object at the manifest root using JSON Merge Patch semantics: object fields merge recursively, null removes the target key, and missing keys are left unchanged. It validates the merged manifest through the node dry-run endpoint, and POSTs it when dry_run=false. Prefer dry_run=true first to preview host-level edits such as enrollment, layer4, tee_backend, trusted_signers, require_scopes, kv_caches, scheduler, telemetry_sample_rate, instance_pool_max_memory_bytes, cloud_sync_endpoint, or batch_targets. The structural fields routes, config_version, and asset_versions cannot be patched here, including via null; use tachyon_patch_route for route changes.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["patch"],
                        "properties": {
                            "patch": {
                                "type": "object",
                                "description": "Manifest-root fragment to merge, such as {\"enrollment\":{\"mode\":\"both\",\"oidc_issuer\":\"https://issuer.example\"},\"require_scopes\":true}. Set a field to null to remove it, for example {\"tee_backend\":null}."
                            },
                            "dry_run": {
                                "type": "boolean",
                                "default": true,
                                "description": "When true, return manifest_preview without posting to the node. Recommended for first use."
                            }
                        }
                    }
                },
                {
                    "name": "tachyon_suggest_scopes",
                    "description": "Suggest a starting scopes configuration for a route based on its current state and lifetime denial count. Returns a conservative YAML snippet and rationale. Apply the suggestion with tachyon_set_route_scopes.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["route_path"],
                        "properties": {
                            "route_path": {
                                "type": "string",
                                "description": "HTTP route path of the deployment to analyse (e.g. '/api/my-fn')."
                            }
                        }
                    }
                },
                {
                    "name": "tachyon_tail_logs",
                    "description": "Fetch the last N log lines from the audit log. Returns a fixed snapshot; continuous streaming is not supported over stdio MCP.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "lines": { "type": "integer", "minimum": 1, "maximum": 1000 }
                        }
                    }
                },
                {
                    "name": "tachyon_get_shadow_diffs",
                    "description": "Return divergence reports produced by system-faas-shadow-proxy.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "tachyon_run_chaos_scenario",
                    "description": "Start a Tachyon chaos harness scenario and return the accepted scenario outcome.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["scenario"],
                        "properties": {
                            "scenario": {
                                "type": "string",
                                "enum": ["network_partition", "pod_eviction", "cpu_pressure", "lora_swap_failure"]
                            },
                            "durationSeconds": { "type": "integer", "minimum": 1, "maximum": 3600 },
                            "target": { "type": "string" }
                        }
                    }
                },
                {
                    "name": "tachyon_hardware_status",
                    "description": "Return local RAM and accelerator availability for sizing Tachyon FaaS manifests.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "validate_faas_capabilities",
                    "description": "Validate a draft FaaS hardware policy against the current local node capacity.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "accelerators": {
                                "type": "array",
                                "items": { "type": "string" }
                            },
                            "minRamMb": { "type": "integer", "minimum": 0 },
                            "minRamGb": { "type": "integer", "minimum": 0 },
                            "minVramMb": { "type": "integer", "minimum": 0 },
                            "qosClass": { "type": "string", "enum": ["realtime", "batch", "background"] },
                            "admissionStrategy": { "type": "string", "enum": ["fail_fast", "mesh_retry"] }
                        }
                    }
                },
                {
                    "name": "tachyon_import_package",
                    "description": "Imports a FaaS package archive (.tar.gz) produced by the Tachyon build pipeline. Uploads every .wasm inside as an asset, then registers the routes declared in the archive's manifest.json — replacing module names with their uploaded asset URIs. The manifest is applied immediately; no separate seal/apply step is needed.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["package_path"],
                        "properties": {
                            "package_path": { "type": "string", "description": "Absolute local file path to the guest-examples.tar.gz (or any compatible FaaS package) on the MCP host machine." }
                        }
                    }
                },
                {
                    "name": "tachyon_deploy_function",
                    "description": "Deploys a pre-compiled WASM artifact to the mesh. You MUST provide the absolute local path to the .wasm file on the host machine where this MCP server is running. The file is read from disk, uploaded as a named asset, and staged in the workload overlay. Call tachyon_seal_overlay then tachyon_apply_manifest to activate.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["function_name", "artifact_path"],
                        "properties": {
                            "function_name": { "type": "string", "description": "Unique identifier for the function, used as its HTTP route alias." },
                            "artifact_path": { "type": "string", "description": "Absolute local file path to the compiled .wasm artifact on the MCP host machine (e.g. '/home/user/project/target/wasm32-wasip2/release/my_fn.wasm')." },
                            "memory_mb":   { "type": "integer", "default": 128, "minimum": 16, "description": "RAM budget for the function in MiB." },
                            "gpu_vram_mb": { "type": "integer", "default": 0,   "minimum": 0,  "description": "Required VRAM in MiB (0 = CPU-only)." }
                        }
                    }
                },
                {
                    "name": "tachyon_list_functions",
                    "description": "List all deployed functions (routes) in the active sealed manifest.",
                    "inputSchema": { "type": "object", "properties": {} }
                },
                {
                    "name": "tachyon_delete_function",
                    "description": "Remove a deployed function from the overlay configuration. Use tachyon_seal_overlay to persist the removal.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["function_name"],
                        "properties": {
                            "function_name": { "type": "string" }
                        }
                    }
                },
                {
                    "name": "tachyon_function_logs",
                    "description": "Fetch recent stdout/stderr log lines for a specific deployed function, filtered by function name.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["function_name"],
                        "properties": {
                            "function_name": { "type": "string" },
                            "lines": { "type": "integer", "default": 100, "minimum": 1, "maximum": 1000 }
                        }
                    }
                },
                {
                    "name": "tachyon_lora_training_status",
                    "description": "Return the current status for a LoRA training job submitted through the tachyon:mesh/training WIT interface.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["job_id"],
                        "properties": {
                            "job_id": { "type": "string", "description": "LoRA training job id, e.g. 'lora-abc123'." }
                        }
                    }
                },
                {
                    "name": "tachyon_kv_get",
                    "description": "Read a value from the distributed KV-Partition V2 store.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["namespace", "key"],
                        "properties": {
                            "namespace": { "type": "string", "description": "Partition namespace (e.g. 'agent-context')." },
                            "key":       { "type": "string" }
                        }
                    }
                },
                {
                    "name": "tachyon_kv_put",
                    "description": "Writes a key-value pair to the distributed KV-Partition V2 store. The value MUST be a valid JSON-stringified representation of your data (e.g. use JSON.stringify or serde_json::to_string). Plain strings must also be quoted.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["namespace", "key", "value"],
                        "properties": {
                            "namespace": { "type": "string", "description": "The KV partition namespace (e.g. 'global', 'auth', 'agent-context')." },
                            "key":       { "type": "string", "description": "Key within the namespace." },
                            "value":     { "type": "string", "description": "JSON-stringified value (e.g. '{\"status\":\"active\",\"count\":3}'). Must be valid UTF-8." }
                        }
                    }
                },
                {
                    "name": "tachyon_kv_delete",
                    "description": "Deletes a key from the distributed KV-Partition V2 store. This operation is permanent — the key cannot be recovered after deletion.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["namespace", "key"],
                        "properties": {
                            "namespace": { "type": "string", "description": "The KV partition namespace." },
                            "key":       { "type": "string", "description": "Key to delete within the namespace." }
                        }
                    }
                },
                {
                    "name": "tachyon_kv_cache_stats",
                    "description": "Return LLM inference KV-cache counters for a configured model from /admin/kv-cache/{model}/stats.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["model"],
                        "properties": {
                            "model": { "type": "string", "description": "Model reference declared in IntegrityConfig.kv_caches[].model_ref." }
                        }
                    }
                },
                {
                    "name": "tachyon_kv_cache_flush",
                    "description": "Flush all LLM inference KV-cache entries for a configured model via /admin/kv-cache/{model}. Rate-limited mutator.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["model"],
                        "properties": {
                            "model": { "type": "string", "description": "Model reference declared in IntegrityConfig.kv_caches[].model_ref." }
                        }
                    }
                },
                {
                    "name": "tachyon_vector_search",
                    "description": "Read-only RAG/vector query. Forwards query, index, and top_k to the configured vector-search route (default /api/guest-rag-vector), returning nearest context and the route's answer without mutating MCP state.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["query", "index", "top_k"],
                        "properties": {
                            "query": { "type": "string", "description": "Natural-language query to embed and search." },
                            "index": { "type": "string", "description": "Vector index name, for example `tenant-kb`." },
                            "top_k": { "type": "integer", "minimum": 1, "maximum": 50, "description": "Maximum number of nearest matches to return." },
                            "route_path": { "type": "string", "default": "/api/guest-rag-vector", "description": "Optional Tachyon route that implements the RAG/vector HTTP contract." },
                            "embedding_model": { "type": "string", "description": "Optional OpenAI-compatible embedding model alias passed to the route." },
                            "chat_model": { "type": "string", "description": "Optional OpenAI-compatible chat model alias passed to the route." },
                            "documents": {
                                "type": "array",
                                "description": "Optional demo documents to ingest before search. Each item has id and text.",
                                "items": {
                                    "type": "object",
                                    "required": ["id", "text"],
                                    "properties": {
                                        "id": { "type": "string" },
                                        "text": { "type": "string" }
                                    }
                                }
                            }
                        }
                    }
                },
                {
                    "name": "tachyon_canary_split",
                    "description": "Adjusts traffic routing weights between the stable and canary versions of a deployed function. Set weight_pct=0 to perform an immediate rollback — all traffic drains back to the stable version and the canary rollout is aborted. Values 1-100 shift that percentage of live traffic to the canary. Use incrementally (e.g. 10→25→50→100) for a safe progressive rollout.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["route_path", "weight_pct"],
                        "properties": {
                            "route_path": { "type": "string", "description": "HTTP route path of the function under canary rollout (e.g. '/api/my-function')." },
                            "weight_pct": { "type": "integer", "minimum": 0, "maximum": 100, "description": "Percentage of traffic to route to the canary version. 0 = abort and roll back; 100 = full promotion." }
                        }
                    }
                },
                {
                    "name": "list_s3_volumes",
                    "description": "List all S3 volumes configured for a FaaS route. Returns bucket, prefix, guest mount path, and read-only flag for each volume.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["route_path"],
                        "properties": {
                            "route_path": { "type": "string", "description": "HTTP route path of the function (e.g. '/api/my-function')." }
                        }
                    }
                },
                {
                    "name": "attach_s3_volume",
                    "description": "Attach an S3 volume to a FaaS route. The S3 bucket contents are downloaded before each invocation and (for read-write volumes) uploaded back after successful execution.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["route_path", "s3_url", "guest_path"],
                        "properties": {
                            "route_path": { "type": "string", "description": "HTTP route path of the target function." },
                            "s3_url": { "type": "string", "description": "S3 URL in the format s3://bucket/prefix (e.g. 's3://my-bucket/datasets/v1')." },
                            "guest_path": { "type": "string", "description": "Absolute path inside the WASM guest where the volume will be mounted (e.g. '/app/data')." },
                            "readonly": { "type": "boolean", "default": false, "description": "When true, guest writes are rejected and nothing is uploaded to S3 after execution." }
                        }
                    }
                },
                {
                    "name": "detach_s3_volume",
                    "description": "Remove an S3 volume from a FaaS route. Identified by its guest mount path. Subsequent invocations will no longer receive that volume.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["route_path", "guest_path"],
                        "properties": {
                            "route_path": { "type": "string", "description": "HTTP route path of the target function." },
                            "guest_path": { "type": "string", "description": "Guest mount path of the S3 volume to remove (e.g. '/app/data')." }
                        }
                    }
                },
                {
                    "name": "list_volume_backups",
                    "description": "List available S3 snapshots for a route volume, newest first.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["route_path", "guest_path"],
                        "properties": {
                            "route_path": { "type": "string", "description": "HTTP route path of the function." },
                            "guest_path": { "type": "string", "description": "Guest mount path of the volume (e.g. '/app/data')." }
                        }
                    }
                },
                {
                    "name": "backup_volume",
                    "description": "Create an S3 snapshot of a route volume. Returns snapshot metadata including snapshot_id, timestamp, and object count.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["route_path", "guest_path"],
                        "properties": {
                            "route_path": { "type": "string", "description": "HTTP route path of the function." },
                            "guest_path": { "type": "string", "description": "Guest mount path of the volume to back up." }
                        }
                    }
                },
                {
                    "name": "restore_volume",
                    "description": "Restore a route volume from a previously created snapshot. Overwrites the current volume contents with the snapshot data.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["route_path", "guest_path", "snapshot_id"],
                        "properties": {
                            "route_path": { "type": "string", "description": "HTTP route path of the function." },
                            "guest_path": { "type": "string", "description": "Guest mount path of the volume to restore." },
                            "snapshot_id": { "type": "string", "description": "Snapshot identifier from list_volume_backups (e.g. 'api_my-fn/app_data/1748000000000')." }
                        }
                    }
                },
                {
                    "name": "recommend_concurrency_policy",
                    "description": "Recommend a concurrency + consistency + coordination configuration for a FaaS route based on a declared usage pattern. Returns mode names, rationale, risk level, and trade-offs so the operator (human or AI) can apply the policy to integrity.lock.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["pattern"],
                        "properties": {
                            "pattern": {
                                "type": "string",
                                "enum": ["batch", "interactive", "stateful", "etl", "scheduler"],
                                "description": "Usage pattern. `batch` = scheduled jobs, `interactive` = low-latency request/response, `stateful` = shared mutable state, `etl` = pipelines with optimistic concurrency, `scheduler` = singleton coordinator."
                            },
                            "writes_shared_state": { "type": "boolean", "default": false, "description": "Set true if invocations may mutate shared volumes or external state." },
                            "requires_ordering": { "type": "boolean", "default": false, "description": "Set true if invocations must be observed in a single linear order." },
                            "max_latency_ms": { "type": "integer", "minimum": 1, "description": "Optional p99 latency budget in milliseconds; influences whether unrestricted mode is recommended." }
                        }
                    }
                },
                {
                    "name": "tachyon_upload_model",
                    "description": "Upload a local AI model to the cluster via the model broker. Point `path` at a complete model directory (weights plus tokenizer.json, and config.json for safetensors) or a single self-contained file on the MCP host machine. The directory is tar+gzip compressed on the fly during upload — no pre-built archive needed — and verified by hash on commit. On success the model is registered automatically and appears in the model list (/ai/v1/models); the alias is derived from the directory/file name.",
                    "inputSchema": {
                        "type": "object",
                        "required": ["path"],
                        "properties": {
                            "path": { "type": "string", "description": "Absolute local path to the model directory (or single file) on the MCP host machine (e.g. '/home/user/models/tinyllama-1.1b')." }
                        }
                    }
                }
            ]
            });
            if MANIFEST_SCHEMA.get().is_none() {
                tools_result["data"] = json!({
                    "warnings": [
                        "Dynamic manifest JSON schema unavailable — connect to a running core-host to enrich tachyon_dryrun_manifest tool definitions."
                    ]
                });
            }
            tools_result
        }
        "tools/call" => {
            let result = handle_tool_call(request.get("params")).await?;
            if result.get("jsonrpc").is_some() && result.get("error").is_some() {
                return Ok(Some(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": result.get("error").cloned().unwrap_or_else(|| json!({
                        "code": -32603,
                        "message": "unknown MCP tool error"
                    })),
                })));
            }
            result
        }
        "ping" => json!({}),
        other => {
            return Ok(Some(json_rpc_error_response(
                id,
                &JsonRpcError {
                    code: -32601,
                    message: format!("unsupported method `{other}`"),
                    data: None,
                },
            )));
        }
    };

    Ok(Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })))
}

async fn validate_request_auth(context: &McpContext) -> Result<()> {
    if !context.enforce_remote_auth {
        return Ok(());
    }
    // Only call set_connection once per process lifetime to avoid an HTTP
    // round-trip on every JSON-RPC message. The global tachyon_client state
    // persists across calls within the same process.
    if CONNECTION_INITIALIZED.get().is_some() {
        return Ok(());
    }
    tachyon_client::set_connection(context.url.clone(), context.token.clone(), None)
        .await
        .map_err(anyhow::Error::msg)
        .context("failed to validate TACHYON_MCP_PAT against TACHYON_MCP_URL")?;
    // Best-effort: fetch and cache the manifest schema so tool definitions are
    // enriched on the first tools/list response.  A failure here is non-fatal.
    match tachyon_client::get_manifest_schema().await {
        Ok(schema) => {
            let _ = MANIFEST_SCHEMA.set(schema);
        }
        Err(e) => {
            warn!(
                "Failed to fetch dynamic manifest schema from core-host: {e:#}. \
                 Falling back to generic object type — agentic manifest generation may be degraded."
            );
        }
    }
    let _ = CONNECTION_INITIALIZED.set(());
    Ok(())
}

/// Reads local hardware status on a blocking thread and returns it as a
/// JSON-RPC tool result. Extracted from the `handle_tool_call` dispatch to
/// keep that function readable and to make this path independently testable.
async fn get_hardware_status() -> Result<Value> {
    let status = tokio::task::spawn_blocking(tachyon_client::read_local_hardware_status)
        .await
        .context("hardware status task panicked")?;
    let body = serde_json::to_string_pretty(&status).context("failed to encode hardware status")?;
    Ok(json!({
        "content": [{ "type": "text", "text": body }]
    }))
}

fn merge_json_object(target: &mut Value, patch: Value) {
    match (target, patch) {
        (Value::Object(target_map), Value::Object(patch_map)) => {
            for (key, patch_value) in patch_map {
                if patch_value.is_null() {
                    target_map.remove(&key);
                } else {
                    match target_map.get_mut(&key) {
                        Some(target_value) => merge_json_object(target_value, patch_value),
                        None => {
                            target_map.insert(key, patch_value);
                        }
                    }
                }
            }
        }
        (target_slot, patch_value) => {
            *target_slot = patch_value;
        }
    }
}

fn validate_route_patch(patch: &Value) -> Result<()> {
    let object = patch.as_object().ok_or_else(|| {
        anyhow::anyhow!("patch must be a JSON object containing route fields to merge")
    })?;
    for field in ["path", "role"] {
        if object.contains_key(field) {
            anyhow::bail!("patch must not modify structural route field `{field}`");
        }
    }
    Ok(())
}

fn validate_manifest_patch(patch: &Value) -> Result<()> {
    let object = patch.as_object().ok_or_else(|| {
        anyhow::anyhow!("patch must be a JSON object containing manifest fields to merge")
    })?;
    for field in ["routes", "config_version", "asset_versions"] {
        if object.contains_key(field) {
            anyhow::bail!(
                "patch must not modify structural manifest field `{field}`; use the dedicated route or manifest apply tools"
            );
        }
    }
    Ok(())
}

async fn handle_tool_call(params: Option<&Value>) -> Result<Value> {
    let name = params
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .context("missing tool name")?;

    // Wrap the entire tool dispatch in a per-request timeout. An `Elapsed` error
    // surfaces as `__TIMEOUT__` via the string sentinel so `JsonRpcError::from_anyhow`
    // can classify it as `-32001 cluster_unreachable`.
    let tool_result = tokio::time::timeout(mcp_timeout(), async {
        handle_tool_dispatch(name, params).await
    })
    .await
    .unwrap_or_else(|_| Err(anyhow::anyhow!("__TIMEOUT__")));

    tool_result.or_else(|err| {
        Ok(json_rpc_error_response(
            None,
            &JsonRpcError::from_anyhow(&err),
        ))
    })
}

async fn handle_tool_dispatch(name: &str, params: Option<&Value>) -> Result<Value> {
    match name {
        "tachyon_mesh_status" => {
            let status = tachyon_client::get_engine_status().await?;
            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": status
                    }
                ]
            }))
        }
        "tachyon_lockfile" => {
            let lockfile = tachyon_client::read_lockfile().await?;
            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": lockfile
                    }
                ]
            }))
        }
        "tachyon_list_resources" => {
            let resources = tachyon_client::read_resources().await?;
            let body =
                serde_json::to_string_pretty(&resources).context("failed to encode resources")?;
            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": body
                    }
                ]
            }))
        }
        "tachyon_register_resource" => {
            let arguments = params
                .and_then(|value| value.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let input: tachyon_client::MeshResourceInput =
                serde_json::from_value(arguments).context("invalid resource input payload")?;
            let resource = tachyon_client::upsert_overlay_resource(input).await?;
            let body = serde_json::to_string_pretty(&resource)
                .context("failed to encode registered resource")?;
            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": format!(
                            "Registered `{name}` in workspace overlay. Pending CLI re-seal of integrity.lock to take effect.\n\n{body}",
                            name = resource.name,
                        )
                    }
                ]
            }))
        }
        "tachyon_upload_model" => {
            let path = params
                .and_then(|value| value.get("arguments"))
                .and_then(|args| args.get("path"))
                .and_then(|value| value.as_str())
                .context("`tachyon_upload_model` requires a string `path` argument")?
                .to_owned();
            let model_path = tachyon_client::push_large_model(&path).await?;
            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": format!(
                            "Uploaded model from `{path}`. The broker is unpacking and registering it; it will appear in the model list (/ai/v1/models).\n\nServer model path: {model_path}"
                        )
                    }
                ]
            }))
        }
        "tachyon_seal_overlay" => {
            let outcome = tachyon_client::seal_overlay().await?;
            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": serde_json::to_string_pretty(&outcome)?
                    }
                ]
            }))
        }
        "tachyon_apply_manifest" => {
            let outcome = tachyon_client::apply_current_manifest().await?;
            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": serde_json::to_string_pretty(&outcome)?
                    }
                ]
            }))
        }
        "tachyon_dryrun_manifest" => {
            let arguments = params
                .and_then(|value| value.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let manifest = arguments
                .get("manifest")
                .cloned()
                .context("missing manifest payload")?;
            let report = tachyon_client::dryrun_manifest(manifest).await?;
            Ok(text_tool_result(&report)?)
        }
        "tachyon_get_metrics" => {
            let metrics = tachyon_client::get_metrics().await?;
            Ok(text_tool_result(&metrics)?)
        }
        "tachyon_get_scope_denials" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let route_path = arguments.get("route_path").and_then(Value::as_str);

            let metrics = tachyon_client::get_metrics()
                .await
                .map_err(|e| anyhow::anyhow!("{e:#}"))?;
            let scope_denial_total = metrics.scope_denial_total;

            let result = if let Some(path) = route_path {
                let config = tachyon_client::get_manifest_config()
                    .await
                    .map_err(|e| anyhow::anyhow!("{e:#}"))?;
                let routes = config
                    .get("routes")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let route = routes.iter().find(|r| {
                    r.get("path")
                        .and_then(Value::as_str)
                        .map(|p| p == path)
                        .unwrap_or(false)
                });
                let allow_all = route.map(|r| {
                    let scopes = r.get("scopes");
                    scopes.is_none()
                        || scopes
                            .and_then(Value::as_str)
                            .map(|s| s == "allow-all")
                            .unwrap_or(false)
                });
                json!({
                    "route_path": path,
                    "scope_denial_total": scope_denial_total,
                    "allow_all": allow_all.unwrap_or(true),
                    "route_found": route.is_some(),
                    "note": "Per-category breakdown available via prometheus: faas_scope_denials_total{deployment,category}"
                })
            } else {
                json!({
                    "scope_denial_total": scope_denial_total,
                    "note": "Per-category breakdown available via prometheus: faas_scope_denials_total{deployment,category}"
                })
            };
            Ok(text_tool_result(&result)?)
        }
        "tachyon_set_route_scopes" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let route_path = arguments
                .get("route_path")
                .and_then(Value::as_str)
                .context("missing route_path")?;
            let scopes = arguments.get("scopes").cloned().context("missing scopes")?;
            let dry_run = arguments
                .get("dry_run")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            let mut config = tachyon_client::get_manifest_config()
                .await
                .map_err(|e| anyhow::anyhow!("{e:#}"))?;

            let routes = config
                .get_mut("routes")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| anyhow::anyhow!("manifest has no routes array"))?;

            let route = routes.iter_mut().find(|r| {
                r.get("path")
                    .and_then(Value::as_str)
                    .map(|p| p == route_path)
                    .unwrap_or(false)
            });

            let route = match route {
                Some(r) => r,
                None => {
                    return Ok(json_rpc_error_response(
                        None,
                        &JsonRpcError::invalid_params(
                            "route not found",
                            json!({
                                "route_path": route_path,
                                "detail": "route not found in manifest — use tachyon_list_functions to list available routes"
                            }),
                        ),
                    ));
                }
            };

            if let Some(obj) = route.as_object_mut() {
                obj.insert("scopes".to_owned(), scopes.clone());
            }

            if dry_run {
                Ok(text_tool_result(&json!({
                    "dry_run": true,
                    "route_path": route_path,
                    "scopes_applied": scopes,
                    "manifest_preview": config,
                }))?)
            } else {
                tachyon_client::apply_manifest_config(config)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e:#}"))?;
                Ok(text_tool_result(&json!({
                    "success": true,
                    "route_path": route_path,
                    "scopes_applied": scopes,
                    "dry_run": false,
                }))?)
            }
        }
        "tachyon_patch_route" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let route_path = arguments
                .get("route_path")
                .and_then(Value::as_str)
                .context("missing route_path")?;
            let patch = arguments.get("patch").cloned().context("missing patch")?;
            let dry_run = arguments
                .get("dry_run")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            validate_route_patch(&patch)?;

            let mut config = tachyon_client::get_manifest_config()
                .await
                .map_err(|e| anyhow::anyhow!("{e:#}"))?;

            let routes = config
                .get_mut("routes")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| anyhow::anyhow!("manifest has no routes array"))?;

            let route = routes.iter_mut().find(|r| {
                r.get("path")
                    .and_then(Value::as_str)
                    .map(|p| p == route_path)
                    .unwrap_or(false)
            });

            let route = match route {
                Some(r) => r,
                None => {
                    return Ok(json_rpc_error_response(
                        None,
                        &JsonRpcError::invalid_params(
                            "route not found",
                            json!({
                                "route_path": route_path,
                                "detail": "route not found in manifest - use tachyon_list_functions to list available routes"
                            }),
                        ),
                    ));
                }
            };

            merge_json_object(route, patch.clone());
            let route_preview = route.clone();
            let validation_report = tachyon_client::dryrun_manifest(config.clone())
                .await
                .context("patched manifest failed dry-run validation")?;

            if dry_run {
                Ok(text_tool_result(&json!({
                    "dry_run": true,
                    "route_path": route_path,
                    "patch_applied": patch,
                    "validation": validation_report,
                    "route_preview": route_preview,
                    "manifest_preview": config,
                }))?)
            } else {
                tachyon_client::apply_manifest_config(config)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e:#}"))?;
                Ok(text_tool_result(&json!({
                    "success": true,
                    "route_path": route_path,
                    "patch_applied": patch,
                    "validation": validation_report,
                    "dry_run": false,
                }))?)
            }
        }
        "tachyon_patch_manifest" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let patch = arguments.get("patch").cloned().context("missing patch")?;
            let dry_run = arguments
                .get("dry_run")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            validate_manifest_patch(&patch)?;

            let mut config = tachyon_client::get_manifest_config()
                .await
                .map_err(|e| anyhow::anyhow!("{e:#}"))?;

            merge_json_object(&mut config, patch.clone());
            let validation_report = tachyon_client::dryrun_manifest(config.clone())
                .await
                .context("patched manifest failed dry-run validation")?;

            if dry_run {
                Ok(text_tool_result(&json!({
                    "dry_run": true,
                    "patch_applied": patch,
                    "validation": validation_report,
                    "manifest_preview": config,
                }))?)
            } else {
                tachyon_client::apply_manifest_config(config)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e:#}"))?;
                Ok(text_tool_result(&json!({
                    "success": true,
                    "patch_applied": patch,
                    "validation": validation_report,
                    "dry_run": false,
                }))?)
            }
        }
        "tachyon_suggest_scopes" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let route_path = arguments
                .get("route_path")
                .and_then(Value::as_str)
                .context("missing route_path")?;

            let (config, metrics) = tokio::try_join!(
                tachyon_client::get_manifest_config(),
                tachyon_client::get_metrics(),
            )?;

            let routes = config
                .get("routes")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let route = routes.iter().find(|r| {
                r.get("path")
                    .and_then(Value::as_str)
                    .map(|p| p == route_path)
                    .unwrap_or(false)
            });
            let route = match route {
                Some(r) => r,
                None => {
                    return Ok(json_rpc_error_response(
                        None,
                        &JsonRpcError::invalid_params(
                            "route not found",
                            json!({
                                "route_path": route_path,
                                "detail": "route not found in manifest — use tachyon_list_functions to list available routes"
                            }),
                        ),
                    ));
                }
            };

            let current_scopes = route.get("scopes");
            let allow_all = current_scopes.is_none()
                || current_scopes
                    .and_then(Value::as_str)
                    .map(|s| s == "allow-all")
                    .unwrap_or(false);

            let current_state = if allow_all {
                "allow-all"
            } else {
                "explicitly-scoped"
            };

            let scope_denial_total = metrics.scope_denial_total;

            let (suggested_scopes_yaml, rationale, conservative_suggestion) = if allow_all
                && scope_denial_total > 0
            {
                (
                    Some(
                        "# Conservative starting point — tighten patterns after observing runtime behaviour\n\
                         scopes:\n  secrets: [\"**\"]\n  kv: [\"**\"]\n  http: [\"**\"]\n\
                         # Remove categories your function does not use"
                            .to_string(),
                    ),
                    format!(
                        "Route is allow-all with {scope_denial_total} lifetime denial(s). \
                         Suggested scopes grant all patterns within each category as a safe starting point. \
                         Restrict patterns progressively using tachyon_set_route_scopes."
                    ),
                    true,
                )
            } else if allow_all {
                (
                    None,
                    "Route is allow-all with 0 recorded denials — no scope violations observed. \
                     Scopes are optional but recommended for defence-in-depth."
                        .to_owned(),
                    false,
                )
            } else {
                (
                    None,
                    "Route already has explicit scopes configured.".to_owned(),
                    false,
                )
            };

            Ok(text_tool_result(&json!({
                "route_path": route_path,
                "current_state": current_state,
                "scope_denial_total": scope_denial_total,
                "suggested_scopes_yaml": suggested_scopes_yaml,
                "rationale": rationale,
                "conservative_suggestion": conservative_suggestion,
                "apply_with": "tachyon_set_route_scopes",
            }))?)
        }
        "tachyon_tail_logs" => {
            let arguments = params
                .and_then(|value| value.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let lines = arguments
                .get("lines")
                .and_then(Value::as_u64)
                .unwrap_or(100)
                .min(1_000) as usize;
            let logs = tachyon_client::tail_logs(lines).await?;
            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": serde_json::to_string_pretty(&logs)?
                    }
                ]
            }))
        }
        "tachyon_get_shadow_diffs" => {
            let diffs = tachyon_client::get_shadow_diffs().await?;
            Ok(text_tool_result(&diffs)?)
        }
        "tachyon_run_chaos_scenario" => {
            let arguments = params
                .and_then(|value| value.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let request: tachyon_client::ChaosScenarioRequest =
                serde_json::from_value(arguments).context("invalid chaos scenario payload")?;
            let outcome = tachyon_client::run_chaos_scenario(request).await?;
            Ok(text_tool_result(&outcome)?)
        }
        "tachyon_hardware_status" => Ok(get_hardware_status().await?),
        "validate_faas_capabilities" => {
            let arguments = params
                .and_then(|value| value.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let policy: tachyon_client::HardwarePolicy =
                serde_json::from_value(arguments).context("invalid hardware policy payload")?;
            let validation = tachyon_client::validate_hardware_policy(&policy);
            let body = serde_json::to_string_pretty(&validation)
                .context("failed to encode capability validation")?;
            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": body
                    }
                ]
            }))
        }
        // ── WASM function lifecycle ───────────────────────────────────────────
        "tachyon_import_package" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let package_path = arguments
                .get("package_path")
                .and_then(Value::as_str)
                .context("missing package_path")?;
            let result = tachyon_client::import_faas_package(package_path).await?;
            Ok(
                json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&result)? }] }),
            )
        }
        "tachyon_deploy_function" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let function_name = arguments
                .get("function_name")
                .and_then(Value::as_str)
                .context("missing function_name")?;
            let artifact_path = arguments
                .get("artifact_path")
                .and_then(Value::as_str)
                .context("missing artifact_path")?;
            let memory_mb = arguments
                .get("memory_mb")
                .and_then(Value::as_u64)
                .unwrap_or(128);
            let gpu_vram_mb = arguments
                .get("gpu_vram_mb")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let wasm_bytes = tokio::fs::read(artifact_path)
                .await
                .with_context(|| format!("cannot read WASM artifact `{artifact_path}`"))?;
            let result =
                tachyon_client::deploy_function(function_name, wasm_bytes, memory_mb, gpu_vram_mb)
                    .await?;
            Ok(
                json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&result)? }] }),
            )
        }
        "tachyon_list_functions" => {
            let functions = tachyon_client::list_functions().await?;
            Ok(
                json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&functions)? }] }),
            )
        }
        "tachyon_delete_function" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let function_name = arguments
                .get("function_name")
                .and_then(Value::as_str)
                .context("missing function_name")?;
            tachyon_client::delete_function(function_name).await?;
            Ok(
                json!({ "content": [{ "type": "text", "text": format!("function `{function_name}` removed from overlay") }] }),
            )
        }
        "tachyon_function_logs" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let function_name = arguments
                .get("function_name")
                .and_then(Value::as_str)
                .context("missing function_name")?;
            let lines = arguments
                .get("lines")
                .and_then(Value::as_u64)
                .unwrap_or(100)
                .min(1_000) as usize;
            let logs = tachyon_client::function_logs(function_name, lines).await?;
            Ok(
                json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&logs)? }] }),
            )
        }

        // ── KV-Partition V2 ───────────────────────────────────────────────────
        "tachyon_lora_training_status" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let job_id = arguments
                .get("job_id")
                .and_then(Value::as_str)
                .context("missing job_id")?;
            let status = tachyon_client::lora_training_status(job_id).await?;
            Ok(text_tool_result(&status)?)
        }
        "tachyon_kv_get" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let namespace = arguments
                .get("namespace")
                .and_then(Value::as_str)
                .context("missing namespace")?;
            let key = arguments
                .get("key")
                .and_then(Value::as_str)
                .context("missing key")?;
            let value = tachyon_client::kv_get(namespace, key).await?;
            let text = match value {
                Some(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                None => "(key not found)".to_owned(),
            };
            Ok(json!({ "content": [{ "type": "text", "text": text }] }))
        }
        "tachyon_kv_put" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let namespace = arguments
                .get("namespace")
                .and_then(Value::as_str)
                .context("missing namespace")?;
            let key = arguments
                .get("key")
                .and_then(Value::as_str)
                .context("missing key")?;
            let value = arguments
                .get("value")
                .and_then(Value::as_str)
                .context("missing value")?;
            tachyon_client::kv_put(namespace, key, value.as_bytes()).await?;
            Ok(
                json!({ "content": [{ "type": "text", "text": format!("written {namespace}/{key}") }] }),
            )
        }
        "tachyon_kv_delete" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let namespace = arguments
                .get("namespace")
                .and_then(Value::as_str)
                .context("missing namespace")?;
            let key = arguments
                .get("key")
                .and_then(Value::as_str)
                .context("missing key")?;
            tachyon_client::kv_delete(namespace, key).await?;
            Ok(
                json!({ "content": [{ "type": "text", "text": format!("deleted {namespace}/{key}") }] }),
            )
        }

        // ── LLM KV-cache admin ────────────────────────────────────────────────
        "tachyon_kv_cache_stats" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let model = arguments
                .get("model")
                .and_then(Value::as_str)
                .context("missing model")?;
            let stats = tachyon_client::kv_cache_stats(model).await?;
            Ok(text_tool_result(&stats)?)
        }
        "tachyon_kv_cache_flush" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let model = arguments
                .get("model")
                .and_then(Value::as_str)
                .context("missing model")?;
            let outcome = tachyon_client::kv_cache_flush(model).await?;
            Ok(text_tool_result(&outcome)?)
        }

        // ── Vector/RAG search ─────────────────────────────────────────────────
        "tachyon_vector_search" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let query = arguments
                .get("query")
                .and_then(Value::as_str)
                .context("missing query")?
                .to_owned();
            let index = arguments
                .get("index")
                .and_then(Value::as_str)
                .context("missing index")?
                .to_owned();
            let top_k = arguments
                .get("top_k")
                .and_then(Value::as_u64)
                .context("missing top_k")?
                .min(50) as u32;
            let route_path = arguments
                .get("route_path")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| env::var("TACHYON_MCP_VECTOR_SEARCH_PATH").ok())
                .unwrap_or_else(|| "/api/guest-rag-vector".to_owned());
            let documents = arguments
                .get("documents")
                .cloned()
                .map(serde_json::from_value)
                .transpose()
                .context("invalid documents payload")?;
            let request = tachyon_client::VectorSearchRequest {
                query,
                index,
                top_k,
                documents,
                embedding_model: arguments
                    .get("embedding_model")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                chat_model: arguments
                    .get("chat_model")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            };
            let result = tachyon_client::vector_search(&route_path, &request).await?;
            Ok(text_tool_result(&result)?)
        }
        // ── Canary traffic split ──────────────────────────────────────────────
        "tachyon_canary_split" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let route_path = arguments
                .get("route_path")
                .and_then(Value::as_str)
                .context("missing route_path")?;
            let weight_pct = arguments
                .get("weight_pct")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(100) as u8;
            tachyon_client::set_canary_split(route_path, weight_pct).await?;
            let msg = if weight_pct == 0 {
                format!("canary rollout for `{route_path}` aborted")
            } else {
                format!("canary weight for `{route_path}` set to {weight_pct}%")
            };
            Ok(json!({ "content": [{ "type": "text", "text": msg }] }))
        }

        // ── S3 FaaS volume management ─────────────────────────────────────────
        "list_s3_volumes" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let route_path = arguments
                .get("route_path")
                .and_then(Value::as_str)
                .context("missing route_path")?;
            let volumes = tachyon_client::list_s3_volumes(route_path).await?;
            Ok(
                json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&volumes)? }] }),
            )
        }
        "attach_s3_volume" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let route_path = arguments
                .get("route_path")
                .and_then(Value::as_str)
                .context("missing route_path")?;
            let s3_url = arguments
                .get("s3_url")
                .and_then(Value::as_str)
                .context("missing s3_url")?;
            let guest_path = arguments
                .get("guest_path")
                .and_then(Value::as_str)
                .context("missing guest_path")?;
            let readonly = arguments
                .get("readonly")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let entry =
                tachyon_client::attach_s3_volume(route_path, s3_url, guest_path, readonly).await?;
            let mode = if readonly { "read-only" } else { "read-write" };
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!(
                        "S3 volume `{s3_url}` attached to route `{route_path}` at `{guest_path}` ({mode}).\n\n{}",
                        serde_json::to_string_pretty(&entry)?
                    )
                }]
            }))
        }
        "detach_s3_volume" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let route_path = arguments
                .get("route_path")
                .and_then(Value::as_str)
                .context("missing route_path")?;
            let guest_path = arguments
                .get("guest_path")
                .and_then(Value::as_str)
                .context("missing guest_path")?;
            tachyon_client::detach_s3_volume(route_path, guest_path).await?;
            Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!(
                        "S3 volume at `{guest_path}` detached from route `{route_path}`. Manifest updated."
                    )
                }]
            }))
        }

        // ── Volume backup management ──────────────────────────────────────────
        "list_volume_backups" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let route_path = arguments
                .get("route_path")
                .and_then(Value::as_str)
                .context("missing route_path")?;
            let guest_path = arguments
                .get("guest_path")
                .and_then(Value::as_str)
                .context("missing guest_path")?;
            let snapshots = tachyon_client::list_volume_backups(route_path, guest_path).await?;
            Ok(
                json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&snapshots)? }] }),
            )
        }
        "backup_volume" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let route_path = arguments
                .get("route_path")
                .and_then(Value::as_str)
                .context("missing route_path")?;
            let guest_path = arguments
                .get("guest_path")
                .and_then(Value::as_str)
                .context("missing guest_path")?;
            let snapshot = tachyon_client::backup_volume(route_path, guest_path).await?;
            Ok(json!({
                "content": [{ "type": "text", "text": format!(
                    "Backup created: snapshot_id={}, {} objects saved.",
                    snapshot.snapshot_id, snapshot.object_count
                ) }]
            }))
        }
        "restore_volume" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let route_path = arguments
                .get("route_path")
                .and_then(Value::as_str)
                .context("missing route_path")?;
            let guest_path = arguments
                .get("guest_path")
                .and_then(Value::as_str)
                .context("missing guest_path")?;
            let snapshot_id = arguments
                .get("snapshot_id")
                .and_then(Value::as_str)
                .context("missing snapshot_id")?;
            tachyon_client::restore_volume(route_path, guest_path, snapshot_id).await?;
            Ok(json!({
                "content": [{ "type": "text", "text": format!(
                    "Volume at `{guest_path}` on route `{route_path}` restored from snapshot `{snapshot_id}`."
                ) }]
            }))
        }

        // ── Concurrency policy recommendation ────────────────────────────────
        "recommend_concurrency_policy" => {
            let arguments = params
                .and_then(|v| v.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let pattern = arguments
                .get("pattern")
                .and_then(Value::as_str)
                .context("missing pattern")?;
            let requirements = tachyon_client::ConcurrencyRequirements {
                writes_shared_state: arguments
                    .get("writes_shared_state")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                requires_ordering: arguments
                    .get("requires_ordering")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                max_latency_ms: arguments.get("max_latency_ms").and_then(Value::as_u64),
            };
            let recommendation =
                tachyon_client::recommend_concurrency_policy(pattern, &requirements);
            Ok(json!({
                "content": [{ "type": "text", "text": serde_json::to_string_pretty(&recommendation)? }]
            }))
        }

        other => Err(anyhow::anyhow!("unsupported tool `{other}`")),
    }
}

/// Returns `None` when the call is permitted, or `Some(JsonRpcError)` with
/// code `-32002` and `retry_after_ms` when the tool's rate-limit is exceeded.
fn check_rate_limit(context: &McpContext, tool_name: &str) -> Result<Option<JsonRpcError>> {
    Ok(context
        .rate_limiter
        .allow(tool_name)?
        .map(JsonRpcError::rate_limited))
}

fn text_tool_result(value: &impl serde::Serialize) -> Result<Value> {
    Ok(json!({
        "content": [
            {
                "type": "text",
                "text": serde_json::to_string_pretty(value)?
            }
        ]
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state_path(name: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "tachyon-mcp-{name}-{}-{}.state",
            std::process::id(),
            unix_now()
        ))
    }

    #[test]
    fn merge_json_object_recursively_updates_nested_route_fields() {
        let mut route = json!({
            "path": "/api/fn",
            "concurrency": {
                "mode": "unrestricted",
                "on_conflict": "reject"
            },
            "env": {
                "A": "1"
            }
        });
        merge_json_object(
            &mut route,
            json!({
                "concurrency": {
                    "mode": "mesh-singleton",
                    "lock_ttl_ms": 5000
                },
                "env": {
                    "B": "2"
                },
                "adapter_id": "tenant-a"
            }),
        );

        assert_eq!(route["concurrency"]["mode"], "mesh-singleton");
        assert_eq!(route["concurrency"]["on_conflict"], "reject");
        assert_eq!(route["concurrency"]["lock_ttl_ms"], 5000);
        assert_eq!(route["env"]["A"], "1");
        assert_eq!(route["env"]["B"], "2");
        assert_eq!(route["adapter_id"], "tenant-a");
    }

    #[test]
    fn merge_json_object_removes_top_level_key_when_patch_value_is_null() {
        let mut route = json!({
            "path": "/api/fn",
            "canary": {
                "stable": "v1",
                "candidate": "v2",
                "weight_pct": 25
            },
            "adapter_id": "tenant-a"
        });

        merge_json_object(&mut route, json!({"canary": null}));

        assert!(route.get("canary").is_none());
        assert_eq!(route["adapter_id"], "tenant-a");
    }

    #[test]
    fn merge_json_object_removes_nested_key_when_patch_value_is_null() {
        let mut route = json!({
            "path": "/api/fn",
            "concurrency": {
                "mode": "mesh-singleton",
                "on_conflict": "queue",
                "lock_ttl_ms": 5000
            }
        });

        merge_json_object(&mut route, json!({"concurrency": {"lock_ttl_ms": null}}));

        assert_eq!(route["concurrency"]["mode"], "mesh-singleton");
        assert_eq!(route["concurrency"]["on_conflict"], "queue");
        assert!(route["concurrency"].get("lock_ttl_ms").is_none());
    }

    #[test]
    fn merge_json_object_null_for_missing_key_is_noop() {
        let mut route = json!({
            "path": "/api/fn",
            "adapter_id": "tenant-a"
        });

        merge_json_object(&mut route, json!({"shadow_target": null}));

        assert_eq!(
            route,
            json!({
                "path": "/api/fn",
                "adapter_id": "tenant-a"
            })
        );
    }

    #[test]
    fn validate_route_patch_rejects_structural_fields() {
        assert!(validate_route_patch(&json!({"concurrency": {"mode": "unrestricted"}})).is_ok());
        assert!(validate_route_patch(&json!({"path": "/api/other"})).is_err());
        assert!(validate_route_patch(&json!({"path": null})).is_err());
        assert!(validate_route_patch(&json!({"role": "system"})).is_err());
        assert!(validate_route_patch(&json!({"role": null})).is_err());
        assert!(validate_route_patch(&json!(["not", "object"])).is_err());
    }

    #[test]
    fn validate_manifest_patch_rejects_structural_fields() {
        assert!(validate_manifest_patch(&json!({
            "enrollment": {"mode": "both"},
            "require_scopes": true,
            "kv_caches": [{"model": "llama-3"}],
        }))
        .is_ok());
        assert!(validate_manifest_patch(&json!({"routes": []})).is_err());
        assert!(validate_manifest_patch(&json!({"routes": null})).is_err());
        assert!(validate_manifest_patch(&json!({"config_version": 2})).is_err());
        assert!(validate_manifest_patch(&json!({"config_version": null})).is_err());
        assert!(validate_manifest_patch(&json!({"asset_versions": {}})).is_err());
        assert!(validate_manifest_patch(&json!({"asset_versions": null})).is_err());
        assert!(validate_manifest_patch(&json!(["not", "object"])).is_err());
    }

    #[test]
    fn patch_manifest_missing_args_and_rate_limit_match_spec() {
        assert_eq!(
            missing_required_args("tachyon_patch_manifest", Some(&json!({}))),
            Some(vec!["patch".to_owned()])
        );
        assert!(missing_required_args(
            "tachyon_patch_manifest",
            Some(&json!({"patch": {"require_scopes": true}}))
        )
        .is_none());

        let spec = rate_limit_spec("tachyon_patch_manifest")
            .expect("patch_manifest must have a rate limit");
        assert_eq!(spec.limit, 1);
    }

    #[tokio::test]
    async fn initialize_round_trips_json_rpc() {
        let context = McpContext::new_for_tests(test_state_path("initialize"));
        let response = handle_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            &context,
        )
        .await
        .expect("initialize should parse")
        .expect("initialize returns a response");

        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["id"], 1);
        assert_eq!(response["result"]["serverInfo"]["name"], "tachyon-mcp");
    }

    #[test]
    fn per_tool_rate_limit_denies_second_apply_manifest_call() {
        let limiter = ToolRateLimiter::new_with_path(test_state_path("apply-limit"));

        assert!(
            limiter
                .allow("tachyon_apply_manifest")
                .expect("first request should succeed")
                .is_none(),
            "first request should not be rate-limited"
        );
        let retry_ms = limiter
            .allow("tachyon_apply_manifest")
            .expect("second request result should not error");
        assert!(retry_ms.is_some(), "second request should be rate-limited");
        assert!(
            limiter
                .allow("tachyon_get_metrics")
                .expect("read tool should have an independent bucket")
                .is_none(),
            "read tool should not be rate-limited"
        );
    }

    #[test]
    fn scope_tools_missing_args_detected() {
        // tachyon_set_route_scopes requires route_path and scopes
        assert!(
            missing_required_args("tachyon_set_route_scopes", Some(&json!({}))).is_some(),
            "empty args must be missing"
        );
        assert!(
            missing_required_args(
                "tachyon_set_route_scopes",
                Some(&json!({"route_path": "/api/fn"}))
            )
            .is_some(),
            "missing scopes must be detected"
        );
        assert!(
            missing_required_args(
                "tachyon_set_route_scopes",
                Some(&json!({"route_path": "/api/fn", "scopes": {}}))
            )
            .is_none(),
            "all required args present should return None"
        );

        // tachyon_suggest_scopes requires route_path
        assert!(
            missing_required_args("tachyon_suggest_scopes", Some(&json!({}))).is_some(),
            "missing route_path must be detected"
        );
        assert!(
            missing_required_args(
                "tachyon_suggest_scopes",
                Some(&json!({"route_path": "/api/fn"}))
            )
            .is_none(),
            "route_path present should return None"
        );

        // tachyon_get_scope_denials has no required args
        assert!(
            missing_required_args("tachyon_get_scope_denials", Some(&json!({}))).is_none(),
            "get_scope_denials has no required args"
        );
    }

    #[test]
    fn kv_cache_tools_require_model_arg() {
        assert_eq!(
            missing_required_args("tachyon_kv_cache_stats", Some(&json!({}))),
            Some(vec!["model".to_owned()])
        );
        assert_eq!(
            missing_required_args("tachyon_kv_cache_flush", Some(&json!({}))),
            Some(vec!["model".to_owned()])
        );
        assert!(missing_required_args(
            "tachyon_kv_cache_stats",
            Some(&json!({"model": "llama-3"}))
        )
        .is_none());
    }

    #[test]
    fn scope_tools_rate_limits_match_spec() {
        let spec_set = rate_limit_spec("tachyon_set_route_scopes");
        assert!(
            spec_set.is_some(),
            "tachyon_set_route_scopes must have a rate limit"
        );
        assert_eq!(
            spec_set
                .expect("set_route_scopes must have a rate limit spec")
                .limit,
            1,
            "set_route_scopes limit must be 1/min"
        );

        let spec_get = rate_limit_spec("tachyon_get_scope_denials");
        assert!(
            spec_get.is_some(),
            "tachyon_get_scope_denials must have a rate limit"
        );
        assert_eq!(
            spec_get
                .expect("get_scope_denials must have a rate limit spec")
                .limit,
            30,
            "get_scope_denials limit must be 30/min"
        );

        let spec_suggest = rate_limit_spec("tachyon_suggest_scopes");
        assert!(
            spec_suggest.is_some(),
            "tachyon_suggest_scopes must have a rate limit"
        );
        assert_eq!(
            spec_suggest
                .expect("suggest_scopes must have a rate limit spec")
                .limit,
            30,
            "suggest_scopes limit must be 30/min"
        );
    }

    #[test]
    fn kv_cache_tool_rate_limits_match_spec() {
        let stats = rate_limit_spec("tachyon_kv_cache_stats")
            .expect("kv-cache stats must have a rate limit");
        assert_eq!(stats.limit, 100);

        let flush = rate_limit_spec("tachyon_kv_cache_flush")
            .expect("kv-cache flush must have a rate limit");
        assert_eq!(flush.limit, 30);
    }

    #[test]
    fn vector_search_required_args_and_rate_limit_match_spec() {
        assert_eq!(
            missing_required_args("tachyon_vector_search", Some(&json!({}))),
            Some(vec![
                "query".to_owned(),
                "index".to_owned(),
                "top_k".to_owned()
            ])
        );
        assert!(missing_required_args(
            "tachyon_vector_search",
            Some(&json!({"query": "q", "index": "tenant-kb", "top_k": 3}))
        )
        .is_none());

        let spec =
            rate_limit_spec("tachyon_vector_search").expect("vector search must have a rate limit");
        assert_eq!(spec.limit, 100);
    }

    #[tokio::test]
    async fn tools_list_includes_kv_cache_admin_tools() {
        let context = McpContext::new_for_tests(test_state_path("tools-list-kv-cache"));
        let response = handle_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            &context,
        )
        .await
        .expect("tools/list should parse")
        .expect("tools/list returns a response");

        let tools = response["result"]["tools"]
            .as_array()
            .expect("tools result should be an array");
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .collect();

        assert!(names.contains(&"tachyon_kv_cache_stats"));
        assert!(names.contains(&"tachyon_kv_cache_flush"));
        assert!(names.contains(&"tachyon_vector_search"));
    }

    #[tokio::test]
    async fn tools_list_includes_patch_manifest_tool() {
        let context = McpContext::new_for_tests(test_state_path("tools-list-patch-manifest"));
        let response = handle_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            &context,
        )
        .await
        .expect("tools/list should parse")
        .expect("tools/list returns a response");

        let tools = response["result"]["tools"]
            .as_array()
            .expect("tools result should be an array");
        let patch_manifest = tools
            .iter()
            .find(|tool| tool.get("name").and_then(Value::as_str) == Some("tachyon_patch_manifest"))
            .expect("patch_manifest tool should be advertised");

        assert_eq!(patch_manifest["inputSchema"]["required"], json!(["patch"]));
        assert_eq!(
            patch_manifest["inputSchema"]["properties"]["dry_run"]["default"],
            true
        );
        assert!(
            patch_manifest["description"]
                .as_str()
                .expect("patch_manifest description should be a string")
                .contains("scheduler"),
            "patch_manifest description should advertise scheduler host-level edits"
        );
    }

    #[test]
    fn rate_limited_response_includes_retry_after_ms() {
        let limiter = ToolRateLimiter::new_with_path(test_state_path("retry-after"));
        let _ = limiter.allow("tachyon_apply_manifest");
        let retry_ms = limiter
            .allow("tachyon_apply_manifest")
            .expect("second call should return retry_after_ms")
            .expect("second call should be denied with retry info");
        // retry_after_ms should be ≤ RATE_LIMIT_WINDOW_SECS * 1000
        assert!(retry_ms <= RATE_LIMIT_WINDOW_SECS * 1_000);
        let err = JsonRpcError::rate_limited(retry_ms);
        assert_eq!(err.code, -32002);
        assert_eq!(
            err.data.expect("rate_limited error has data")["retry_after_ms"],
            retry_ms
        );
    }

    #[tokio::test]
    async fn tool_call_returns_json_rpc_rate_limit_error() {
        let context = McpContext::new_for_tests(test_state_path("rpc-limit"));
        let request = r#"{"jsonrpc":"2.0","id":"a","method":"tools/call","params":{"name":"tachyon_apply_manifest","arguments":{}}}"#;

        // Consume the one allowed call.
        let _ = context
            .rate_limiter
            .allow("tachyon_apply_manifest")
            .expect("preflight call should consume the apply_manifest bucket");
        let denied = handle_line(request, &context)
            .await
            .expect("second call should produce a JSON-RPC response")
            .expect("rate-limited request returns a response");

        assert_eq!(denied["jsonrpc"], "2.0");
        assert_eq!(denied["id"], "a");
        assert_eq!(denied["error"]["code"], -32002);
        assert!(denied["error"]["data"]["retry_after_ms"].is_number());
    }
}
