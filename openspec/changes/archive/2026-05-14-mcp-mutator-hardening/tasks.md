# Implementation Tasks

- [x] **Task 1: Mutator Rate Limits**
  - Locate the rate limit logic in `tachyon-mcp/src/main.rs` (lines ~270-281).
  - Implement a match statement or configuration map to apply the granular limits (2/min for canary, 5/min for deploy, etc.).
  - Verify it correctly returns the `-32002` JsonRpcError.

- [x] **Task 2: KV Schema Fixes**
  - Update `tachyon_kv_put` and `tachyon_kv_delete` schemas in `main.rs` (lines ~663-695).
  - Add the missing `required: ["namespace", "key"]` (and `value` for put) arrays.

- [x] **Task 3: LLM Descriptions**
  - Update the `description` fields for `deploy_function`, `kv_put`, and `canary_split` to explicitly guide the LLM on data formats (.wasm local path, stringified JSON, rollback semantics).

- [x] **Task 4: Dead Code Cleanup (Bonus P2)**
  - While in `main.rs`, delete the obsolete `error_response()` function (lines ~1144-1149) in favor of the standardized `json_rpc_error_response()`.
