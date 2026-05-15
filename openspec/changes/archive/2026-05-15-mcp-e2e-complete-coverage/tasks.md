# Implementation Tasks

- [x] **Task 1: Setup Test Artifacts**
  - Update `tachyon-mcp/tests/mcp_e2e_runner.rs` to include the `create_dummy_wasm()` helper.

- [x] **Task 2: Implement Lifecycle Tests**
  - Add the JSON-RPC request/response assertion blocks for `tachyon_deploy_function`, `tachyon_list_functions`, `tachyon_function_logs`, and `tachyon_delete_function`.
  
- [x] **Task 3: Implement KV & Error Tests**
  - Add the assertion blocks for `tachyon_kv_get` and `tachyon_kv_delete`.
  - Add a block that intentionally triggers a validation error (e.g., calling `deploy_function` without an `artifact_path`) and strictly asserts the `-32602` error code.

- [x] **Task 4: Run E2E Suite**
  - Run `cargo test --package tachyon-mcp` locally (ensuring a local `core-host` is active if the tests require it) to verify the new suite passes seamlessly without flakiness.