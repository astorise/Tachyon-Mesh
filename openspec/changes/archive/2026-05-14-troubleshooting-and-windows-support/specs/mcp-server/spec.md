# Technical Specification: MCP Schema Fetch Resilience

## 1. Schema Fetch Logging (`tachyon-mcp/src/main.rs`)
Locate the initialization of `MANIFEST_SCHEMA` (around lines ~763).

**Before:**
```rust
let _ = MANIFEST_SCHEMA.set(fetched_schema);
```

**After:**
```rust
use tracing::warn;

if let Err(_) = MANIFEST_SCHEMA.set(fetched_schema) {
    warn!("Failed to fetch dynamic manifest schema from core-host. Falling back to generic object type. Agentic manifest generation may be degraded.");
}
```

## 2. Exposing Warnings in `tools/list`
Update the `tools/list` JSON-RPC handler to inspect the state of `MANIFEST_SCHEMA`. If it is empty or failed, append a metadata warning.

```rust
// In the tools/list handler
let mut response = json!({
    "tools": get_tool_definitions()
});

if MANIFEST_SCHEMA.get().is_none() {
    response["data"] = json!({
        "warnings": ["Dynamic manifest JSON schema is unavailable. Manifest validation must be guessed."]
    });
}

Ok(response)
```