# Technical Specification: MCP Unblocking & Core Fixes

## 1. Documentation (`docs/mcp-setup.md` & `README.md`)
Create a new file `docs/mcp-setup.md` with instructions for standard LLM clients.

```json
// Example for claude_desktop_config.json
{
  "mcpServers": {
    "tachyon-mesh": {
      "command": "cargo",
      "args": ["run", "--bin", "tachyon-mcp"],
      "env": {
        "TACHYON_MCP_URL": "[http://127.0.0.1:8080](http://127.0.0.1:8080)",
        "TACHYON_MCP_PAT": "your-personal-access-token"
      }
    }
  }
}
```
*Requirement: Add a prominent section in the root `README.md` pointing to this setup guide.*

## 2. Tokio Executor Unblocking (`tachyon-mcp/src/main.rs`)
**Target:** Line ~346.
Currently, the hardware status is read synchronously. Update the client library call (if necessary) and the MCP handler to be asynchronous.

```rust
// DANGEROUS: Bloque le thread OS
// let status = tachyon_client::read_local_hardware_status();

// SAFE: Libère l'executor
let status = tachyon_client::read_local_hardware_status_async().await
    .map_err(|e| json_rpc_error(-32603, e.to_string()))?;
```

## 3. Connection Caching (`tachyon-mcp/src/main.rs`)
**Target:** Line ~541.
Currently, `set_connection()` is called on every request. Refactor the MCP server to hold a persistent client state.

```rust
use std::sync::OnceLock;
use tokio::sync::RwLock;

static TACHYON_CLIENT: OnceLock<RwLock<tachyon_client::Client>> = OnceLock::new();

// Initialize once during MCP startup
async fn get_or_init_client() -> Result<tachyon_client::Client, Error> {
    // Implement standard lazy initialization with TACHYON_MCP_URL
}
```

## 4. Contract Alignment: `tachyon_tail_logs`
**Target:** Line ~668.
Modify the tool schema definition for `tachyon_tail_logs` to remove the `follow` property entirely, as standard stdio-based MCP does not easily support long-lived streaming log pipes without specific chunking logic. 

```rust
// Remove from JSON schema:
// "follow": { "type": "boolean", "description": "Stream logs continuously" }

// Return a fixed chunk of the last N lines instead.
```