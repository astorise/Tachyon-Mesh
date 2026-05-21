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
        "tachyon_deploy_function" => &["function_name", "artifact_path"],
        "tachyon_delete_function" => &["function_name"],
        "tachyon_function_logs" => &["function_name"],
        "tachyon_kv_get" => &["namespace", "key"],
        "tachyon_kv_put" => &["namespace", "key", "value"],
        "tachyon_kv_delete" => &["namespace", "key"],
        "tachyon_canary_split" => &["route_path", "weight_pct"],
        "tachyon_register_resource" => &["name", "type", "target"],
        "tachyon_dryrun_manifest" => &["manifest"],
        "tachyon_run_chaos_scenario" => &["scenario"],
        "list_s3_volumes" => &["route_path"],
        "attach_s3_volume" => &["route_path", "s3_url", "guest_path"],
        "detach_s3_volume" => &["route_path", "guest_path"],
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
        "tachyon_apply_manifest" | "tachyon_seal_overlay" => 1,
        "tachyon_deploy_function" | "tachyon_delete_function" => 5,
        "tachyon_register_resource" => 10,
        // KV mutators and log fetches — generous but bounded.
        "tachyon_kv_put" | "tachyon_kv_delete" | "tachyon_function_logs" => 30,
        // Read-only telemetry tools.
        "tachyon_get_metrics" | "tachyon_tail_logs" => 30,
        // S3 volume mutators — moderate budget.
        "attach_s3_volume" | "detach_s3_volume" => 10,
        // All remaining read-only tools — high throughput allowed.
        "tachyon_mesh_status"
        | "tachyon_lockfile"
        | "tachyon_topology_snapshot"
        | "tachyon_hardware_status"
        | "tachyon_list_resources"
        | "tachyon_list_functions"
        | "tachyon_kv_get"
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
                    "description": "Return active node telemetry such as error rate, p50/p99 latency, and queue depth.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
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
            Ok(json!({ "content": [{ "type": "text", "text": serde_json::to_string_pretty(&volumes)? }] }))
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
