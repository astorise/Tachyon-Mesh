# Technical Specification: Advanced MCP Tools

## 1. Tool Schemas (`tachyon-mcp/src/main.rs`)
Register the new tools in the `mcp.list_tools` JSON-RPC response.

### Example: `tachyon_deploy_function`
```json
{
  "name": "tachyon_deploy_function",
  "description": "Deploys a pre-compiled WASM artifact to the mesh.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "function_name": { "type": "string" },
      "artifact_path": { "type": "string", "description": "Local path to the .wasm file" },
      "memory_mb": { "type": "integer", "default": 128 },
      "gpu_vram_mb": { "type": "integer", "default": 0, "description": "Required VRAM if AI model attached" }
    },
    "required": ["function_name", "artifact_path"]
  }
}
```

### Example: `tachyon_function_logs`
```json
{
  "name": "tachyon_function_logs",
  "description": "Fetches the recent stdout/stderr logs for a specific deployed function.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "function_name": { "type": "string" },
      "lines": { "type": "integer", "default": 100 }
    },
    "required": ["function_name"]
  }
}
```

### Example: `tachyon_kv_put`
```json
{
  "name": "tachyon_kv_put",
  "description": "Writes a key-value pair to the distributed KV-Partition V2 store.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "namespace": { "type": "string" },
      "key": { "type": "string" },
      "value": { "type": "string", "description": "JSON stringified value" }
    },
    "required": ["namespace", "key", "value"]
  }
}
```

## 2. Tool Handlers Implementation
Add matching asynchronous handlers in the MCP request router.

```rust
// Inside the main RPC match block
"tachyon_deploy_function" => {
    let params: DeployParams = serde_json::from_value(request.params)?;
    // Read the WASM file from disk (MCP server runs locally alongside the agent)
    let wasm_bytes = tokio::fs::read(&params.artifact_path).await
        .map_err(|e| JsonRpcError::invalid_params("Cannot read artifact", json!({"error": e.to_string()})))?;
    
    let res = tachyon_client::deploy_function(&params.function_name, wasm_bytes, params.memory_mb, params.gpu_vram_mb).await?;
    Ok(json!({ "status": "deployed", "details": res }))
}
"tachyon_kv_get" => {
    let params: KvParams = serde_json::from_value(request.params)?;
    let res = tachyon_client::kv_get(&params.namespace, &params.key).await?;
    Ok(json!({ "value": res }))
}
// ... implement others similarly
```