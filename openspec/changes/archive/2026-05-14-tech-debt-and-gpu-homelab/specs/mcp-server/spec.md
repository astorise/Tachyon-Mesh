# Technical Specification: MCP Refactoring

## 1. Deduplicate Hardware Polling
In `tachyon-mcp/src/main.rs` (lines ~461 and ~929), extract the `spawn_blocking` call into a reusable async helper.

```rust
async fn get_hardware_status() -> Result<HardwareStatus, JsonRpcError> {
    tokio::task::spawn_blocking(|| {
        tachyon_client::read_local_hardware_status()
    })
    .await
    .map_err(|e| JsonRpcError::internal_error(&format!("Task paniced: {}", e)))?
    .map_err(|e| JsonRpcError::internal_error(&format!("Hardware read failed: {}", e)))
}
```

## 2. Enforce Connection Caching
In `tachyon-mcp/src/main.rs`, update the request loop to respect the `CONNECTION_INITIALIZED` state.

```rust
if !CONNECTION_INITIALIZED.load(Ordering::Relaxed) {
    tachyon_client::set_connection(&url, &pat).await?;
    CONNECTION_INITIALIZED.store(true, Ordering::Relaxed);
}
// Skip setting connection on subsequent loop iterations
```

## 3. Dead Code Removal
Delete the legacy `error_response()` function (lines ~1144-1149). Ensure all endpoints use the newly introduced `JsonRpcError` struct and `json_rpc_error_response()` helper.