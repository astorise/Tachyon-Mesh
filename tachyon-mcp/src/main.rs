use anyhow::{Context, Result};
use serde_json::{json, Value};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::main]
async fn main() -> Result<()> {
    let stdin = BufReader::new(io::stdin());
    let mut lines = stdin.lines();
    let mut stdout = io::stdout();

    while let Some(line) = lines.next_line().await.context("failed to read stdin")? {
        if line.trim().is_empty() {
            continue;
        }

        match handle_line(&line).await {
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

async fn handle_line(line: &str) -> Result<Option<Value>> {
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
                "tools": {}
            }
        }),
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
                    "description": "Register a new mesh resource into the workspace overlay (tachyon.resources.json). The entry is persisted as `pending` and requires a CLI re-seal to take effect inside integrity.lock.",
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
                }
            ]
        }),
        "tools/call" => handle_tool_call(request.get("params")).await?,
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

async fn handle_tool_call(params: Option<&Value>) -> Result<Value> {
    let name = params
        .and_then(|value| value.get("name"))
        .and_then(Value::as_str)
        .context("missing tool name")?;

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
        other => Ok(error_response(
            None,
            -32602,
            &format!("unsupported tool `{other}`"),
        )),
    }
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
