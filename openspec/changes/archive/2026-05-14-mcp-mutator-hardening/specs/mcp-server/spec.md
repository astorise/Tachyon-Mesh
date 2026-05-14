# Technical Specification: MCP Hardening

## 1. Tool-Specific Rate Limits (`tachyon-mcp/src/main.rs`)
Update the rate-limiting middleware to apply different token bucket configurations based on the tool name.

```rust
// Suggested Configuration Map:
// - tachyon_canary_split: 2 requests per 60s
// - tachyon_deploy_function: 5 requests per 60s
// - tachyon_delete_function: 5 requests per 60s
// - tachyon_kv_put, tachyon_kv_delete: 30 requests per 60s
// - tachyon_function_logs: 30 requests per 60s
// - all read-only tools: 100 requests per 60s

// If exceeded, return the standardized -32002 error with retry_after_ms.
```

## 2. Schema Enrichment: `tachyon_kv_put`
Ensure `required` fields are strictly defined and the value format is explicitly stated.

```json
{
  "name": "tachyon_kv_put",
  "description": "Writes a key-value pair to the distributed KV-Partition V2 store. The value MUST be a valid JSON stringified representation of your data.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "namespace": { "type": "string", "description": "The KV namespace (e.g., 'global', 'auth')" },
      "key": { "type": "string" },
      "value": { "type": "string", "description": "JSON stringified value (e.g., '{\"status\":\"active\"}')" }
    },
    "required": ["namespace", "key", "value"]
  }
}
```

## 3. Schema Enrichment: `tachyon_deploy_function`
Clarify the artifact sourcing mechanism.

```json
{
  "name": "tachyon_deploy_function",
  "description": "Deploys a pre-compiled WASM artifact to the mesh. You MUST provide the absolute local path to the .wasm file on the host machine where this MCP server is running.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "function_name": { "type": "string", "description": "Unique identifier for the function" },
      "artifact_path": { "type": "string", "description": "Absolute local file path to the compiled .wasm artifact" },
      "memory_mb": { "type": "integer", "default": 128 },
      "gpu_vram_mb": { "type": "integer", "default": 0 }
    },
    "required": ["function_name", "artifact_path"]
  }
}
```

## 4. Schema Enrichment: `tachyon_canary_split`
```json
{
  "name": "tachyon_canary_split",
  "description": "Adjusts traffic routing weights between versions. Set weight_pct=0 to perform an immediate rollback/traffic drain.",
  // ... properties and required array
}
```