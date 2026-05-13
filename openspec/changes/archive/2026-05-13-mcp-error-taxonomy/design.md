# Design: MCP Error Taxonomy

## Approach

All four changes land in `tachyon-mcp/src/main.rs`. No external dependencies are added.

### 1. `JsonRpcError` struct and constructors

Four factory methods map directly to the spec's error codes:

| Constructor | Code | Use case |
|---|---|---|
| `invalid_params(msg, details)` | -32602 | Schema/validation failures; `details` carries structured context |
| `cluster_unreachable(msg)` | -32001 | Timeout or connection refused reaching core-host |
| `rate_limited(retry_after_ms)` | -32002 | Bucket exhausted; `data.retry_after_ms` tells the agent when to retry |
| `internal_error(msg)` | -32603 | Unexpected failures |

`JsonRpcError::from_anyhow` classifies `anyhow::Error` by scanning the error chain for well-known substrings: `__TIMEOUT__` / `timed out` / `deadline` → `-32001`; `connection refused` / `failed to connect` → `-32001`; `validation` / `invalid` / `schema` → `-32602`; everything else → `-32603`.

A companion `json_rpc_error_response(id, &err)` produces the full JSON-RPC 2.0 envelope.

### 2. Rate limiter — `retry_after_ms`

`TokenBucket::allow` return type changed from `bool` to `Option<u64>` (`None` = allowed, `Some(ms)` = denied with retry delay). The delay is `(last_refill_unix + window_secs - now) * 1000`, clamped to 0 by `saturating_sub`. `ToolRateLimiter::allow` propagates the `Option<u64>`. The call site in `handle_tool_call` calls `check_rate_limit` which wraps a denial in `JsonRpcError::rate_limited(ms)` and returns it immediately, so the response code changes from `-32000` (non-standard) to `-32002` with a `data.retry_after_ms` field.

### 3. Timeout enforcer

`mcp_timeout()` reads `TACHYON_MCP_TIMEOUT_MS` from the environment (default 5 000 ms). The entire `handle_tool_dispatch` async block is wrapped in `tokio::time::timeout`. An `Elapsed` error sets the sentinel `__TIMEOUT__` on the error chain, which `from_anyhow` maps to `-32001`.

### 4. Error routing in `handle_line`

Auth failures (`validate_request_auth`) now use `JsonRpcError::cluster_unreachable` instead of the flat `-32603`. Unsupported resource URIs and unsupported methods use `json_rpc_error_response` directly. The legacy `error_response` helper is retained for the few call-sites in the initialization path where the `JsonRpcError` type would add no value.

## Trade-offs

| Decision | Chosen | Rejected | Reason |
|---|---|---|---|
| Error classification scope | `from_anyhow` at `handle_tool_call` boundary | Per-call inline mapping | Avoids touching 14 call-sites; the sentinel pattern is unambiguous for timeout |
| Timeout placement | Wrap entire `handle_tool_dispatch` | Wrap individual `tachyon_client` calls | One timeout guards the whole exchange; per-call wrapping would require 14 identical edits |
| Old `-32000` rate-limit code | Replaced by `-32002` | Keep `-32000` | `-32002` is the semantic home for rate limits in this taxonomy; `-32000` was informal |
| `error_response` helper | Retained | Fully removed | Still used in main loop init path where no structured type adds value |
