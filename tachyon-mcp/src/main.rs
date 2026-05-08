use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{
    env,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

const WRITE_LIMIT_PER_MINUTE: u32 = 5;

struct McpContext {
    _token: String,
    write_limiter: Mutex<TokenBucket>,
}

struct TokenBucket {
    tokens: u32,
    last_refill: Instant,
}

impl TokenBucket {
    fn new() -> Self {
        Self {
            tokens: WRITE_LIMIT_PER_MINUTE,
            last_refill: Instant::now(),
        }
    }

    fn allow(&mut self) -> bool {
        if self.last_refill.elapsed() >= Duration::from_secs(60) {
            self.tokens = WRITE_LIMIT_PER_MINUTE;
            self.last_refill = Instant::now();
        }
        if self.tokens == 0 {
            return false;
        }
        self.tokens -= 1;
        true
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let context = McpContext {
        _token: load_required_token()?,
        write_limiter: Mutex::new(TokenBucket::new()),
    };
    if let Ok(url) = env::var("TACHYON_MCP_URL") {
        tachyon_client::set_connection(url, context._token.clone(), None)
            .await
            .map_err(anyhow::Error::msg)
            .context("failed to validate TACHYON_MCP_PAT against TACHYON_MCP_URL")?;
    }

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
                let response = error_response(None, -32603, &error.to_string());
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
                }
            ]
        }),
        "resources/read" => {
            let uri = request
                .get("params")
                .and_then(|value| value.get("uri"))
                .and_then(Value::as_str)
                .context("missing resource uri")?;
            if uri != "hardware://local/status" {
                return Ok(Some(error_response(
                    id,
                    -32602,
                    &format!("unsupported resource `{uri}`"),
                )));
            }
            let status = tachyon_client::read_local_hardware_status();
            json!({
                "contents": [
                    {
                        "uri": uri,
                        "mimeType": "application/json",
                        "text": serde_json::to_string_pretty(&status)?
                    }
                ]
            })
        }
        "tools/list" => json!({
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
                            "manifest": {
                                "type": "object",
                                "description": "Either a sealed manifest with configPayload/config_payload, or the raw config payload object."
                            }
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
                    "description": "Fetch recent logs and expose them as MCP notifications/message payloads for clients that want to stream them.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "lines": { "type": "integer", "minimum": 1, "maximum": 1000 },
                            "follow": { "type": "boolean" }
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
                }
            ]
        }),
        "tools/call" => handle_tool_call(request.get("params"), context).await?,
        "ping" => json!({}),
        other => {
            return Ok(Some(error_response(
                id,
                -32601,
                &format!("unsupported method `{other}`"),
            )));
        }
    };

    Ok(Some(json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })))
}

async fn handle_tool_call(params: Option<&Value>, context: &McpContext) -> Result<Value> {
    let name = params
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .context("missing tool name")?;

    match name {
        "tachyon_register_resource" | "tachyon_seal_overlay" | "tachyon_apply_manifest"
            if !allow_write(context) =>
        {
            return Ok(error_response(None, -32000, "Rate limit exceeded"));
        }
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
            let follow = arguments
                .get("follow")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let logs = tachyon_client::tail_logs(lines).await?;
            let notifications: Vec<Value> = logs
                .iter()
                .map(|line| {
                    json!({
                        "jsonrpc": "2.0",
                        "method": "notifications/message",
                        "params": {
                            "level": line.level,
                            "logger": line.target,
                            "data": line
                        }
                    })
                })
                .collect();
            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": serde_json::to_string_pretty(&logs)?
                    }
                ],
                "structuredContent": {
                    "followRequested": follow,
                    "notifications": notifications
                }
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
        "tachyon_hardware_status" => {
            let status = tachyon_client::read_local_hardware_status();
            let body = serde_json::to_string_pretty(&status)
                .context("failed to encode hardware status")?;
            Ok(json!({
                "content": [
                    {
                        "type": "text",
                        "text": body
                    }
                ]
            }))
        }
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
        other => Ok(error_response(
            None,
            -32602,
            &format!("unsupported tool `{other}`"),
        )),
    }
}

fn allow_write(context: &McpContext) -> bool {
    context
        .write_limiter
        .lock()
        .expect("write limiter should not be poisoned")
        .allow()
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

fn error_response(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}
