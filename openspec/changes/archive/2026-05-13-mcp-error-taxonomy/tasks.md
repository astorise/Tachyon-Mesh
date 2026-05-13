# Implementation Tasks

- [x] **Task 1: Define `JsonRpcError` Struct**
  - Implement the structured error types and helper constructors in `tachyon-mcp/src/main.rs`.

- [x] **Task 2: Update Tool Handlers**
  - Refactor all 14 tool handlers (`tachyon_register_resource`, `tachyon_apply_manifest`, etc.) to map their internal `Result::Err` to the appropriate `JsonRpcError` instead of a flat string or generic `-32603`.

- [x] **Task 3: Rate Limiter Integration**
  - Update the local MCP rate limiting logic (currently using `/tmp/tachyon-mcp-rate-limits.state`) to calculate the remaining time until reset.
  - Return `-32002` with `retry_after_ms` in the `data` field when triggered.

- [x] **Task 4: Timeout Enforcer**
  - Read `TACHYON_MCP_TIMEOUT_MS` from the environment.
  - Wrap `tachyon_client` calls in `tokio::time::timeout`.
  - Return `-32001` on timeout.