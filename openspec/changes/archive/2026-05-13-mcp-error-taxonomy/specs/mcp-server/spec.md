# Technical Specification: MCP Error Taxonomy

## 1. Error Definitions (`tachyon-mcp/src/main.rs`)
Refactor the error handling logic to support a rich error structure that serializes to the JSON-RPC 2.0 specification.

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Debug)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcError {
    pub fn invalid_params(msg: &str, details: Value) -> Self {
        Self { code: -32602, message: msg.to_string(), data: Some(details) }
    }

    pub fn cluster_unreachable(msg: &str) -> Self {
        Self { code: -32001, message: msg.to_string(), data: None }
    }

    pub fn rate_limited(retry_after_ms: u64) -> Self {
        Self { 
            code: -32002, 
            message: "Rate limit exceeded".to_string(), 
            data: Some(serde_json::json!({ "retry_after_ms": retry_after_ms })) 
        }
    }

    pub fn internal_error(msg: &str) -> Self {
        Self { code: -32603, message: msg.to_string(), data: None }
    }
}
```

## 2. Tool Implementation Updates
Update the tool execution handlers to map internal `tachyon_client` errors to these specific `JsonRpcError` variants.

**Example for Manifest Validation:**
```rust
// Inside tachyon_apply_manifest handler
match tachyon_client::apply_manifest(&payload).await {
    Ok(res) => /* ... */,
    Err(e) => {
        if e.is_validation_error() {
            return Err(JsonRpcError::invalid_params(
                "Manifest validation failed",
                serde_json::json!({ "validation_errors": e.get_structured_errors() })
            ));
        } else if e.is_network_timeout() {
            return Err(JsonRpcError::cluster_unreachable("Timeout connecting to core-host"));
        }
        return Err(JsonRpcError::internal_error(&e.to_string()));
    }
}
```

## 3. Timeout Configuration
Add a configurable timeout for all MCP -> Core-Host requests to ensure we return `-32001` instead of hanging indefinitely.
```rust
// Use env var TACHYON_MCP_TIMEOUT_MS, default to 5000ms
```