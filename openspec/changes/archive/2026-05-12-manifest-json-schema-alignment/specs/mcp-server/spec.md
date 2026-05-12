# Technical Specification: MCP Tool Alignment

## 1. Dynamic Schema Injection
In `tachyon-mcp/src/main.rs`, update the MCP server initialization to fetch the JSON Schema from the `core-host` API.

```rust
// Fetch schema once at MCP startup
let manifest_schema = tachyon_client::get_manifest_schema().await?;

// Inject dynamically into the tool response
"inputSchema": {
    "type": "object",
    "properties": {
        "manifest": manifest_schema
    },
    "required": ["manifest"]
}
```
*Note: This directly replaces the current empty `{"type": "object"}`.*