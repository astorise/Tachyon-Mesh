# Design: MCP E2E Complete Coverage

## What Was Built

End-to-end behavioral coverage for the 7 advanced MCP mutator/reader tools, plus a structured pre-auth parameter validator that gives agents the exact JSON-RPC error code they need to self-correct.

### Task 1 — `create_dummy_wasm()` helper (`mcp_e2e_runner.rs`)
- Writes a minimal valid WASM file (magic `\0asm` + version `\x01\0\0\0`) into `std::env::temp_dir()`.
- Path is unique per-pid + per-nanosecond so concurrent tests cannot collide.
- Returns a `PathBuf` so callers can format it into JSON arguments and clean it up at the end.
- Cross-platform: uses `std::env::temp_dir()` rather than the hardcoded `/tmp` in the spec sketch.

### Task 2 — FaaS lifecycle tests
- `test_function_lifecycle_is_valid_jsonrpc` exercises `tachyon_deploy_function`, `tachyon_list_functions`, `tachyon_function_logs`, and `tachyon_delete_function` in sequence.
- Each response is asserted via the shared `assert_offline_jsonrpc()` helper which checks:
  - `jsonrpc == "2.0"` and `id` is echoed back
  - If an `error.code` is returned, it MUST NOT be `-32603 internal_error` — only `[-32001, -32002, -32602]` are acceptable offline
- Spawned via a new `spawn_offline_mcp()` helper that initialises the MCP server once and returns `(child, stdin, stdout reader)` for reuse across the sequence.

### Task 3 — KV + Error tests
- `test_kv_get_and_delete_are_valid_jsonrpc` covers the two previously-untested KV tools using the same offline harness.
- `test_deploy_function_missing_param_returns_32602` sends a deploy request without `artifact_path` and strictly asserts `error.code == -32602`. This caught a real bug — see "Pre-auth parameter validation" below.

### Task 4 — Test suite execution
- `cargo test -p tachyon-mcp --tests`: **12/12 green** (4 unit + 8 e2e).
- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.

## Source Fix Required by Task 3 — Pre-auth Parameter Validation

The naive E2E test exposed a layering bug in `tachyon-mcp`: required-field validation happened inside each tool's dispatch handler (via `.context("missing X")?`), but `validate_request_auth()` ran BEFORE dispatch. So a malformed offline request returned `-32001 cluster_unreachable` — the agent had no way to know the request was malformed vs. the cluster was down.

**Fix**: Introduced `missing_required_args(tool_name, arguments)` in `main.rs:rate_limit_spec` neighbourhood. The function declares the required-field map (mirroring the `inputSchema.required` arrays) and is called in `handle_request` **before** `validate_request_auth`. Missing fields now surface as a precise `-32602 invalid_params` with `data.tool` and `data.missing` populated for agent self-correction.

Also tightened `JsonRpcError::from_anyhow` to classify `.context("missing X")` propagation as `-32602` rather than `-32603`, so any future tool handler that uses the existing idiom gets the correct code automatically.

## Files Changed
- `tachyon-mcp/src/main.rs` — pre-auth `missing_required_args()` + `from_anyhow` classifier extension
- `tachyon-mcp/tests/mcp_e2e_runner.rs` — `create_dummy_wasm()`, `spawn_offline_mcp()`, `assert_offline_jsonrpc()`, plus 3 new tests
