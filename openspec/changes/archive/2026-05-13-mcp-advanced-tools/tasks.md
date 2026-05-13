# Implementation Tasks

- [x] **Task 1: Define Schemas**
  - Update the `list_tools` endpoint in `tachyon-mcp/src/main.rs` to include the JSON schemas for the 8 new tools (`deploy`, `list`, `delete`, `logs`, `kv_get`, `kv_put`, `kv_delete`, `canary_split`).

- [x] **Task 2: Implement WASM Lifecycle Handlers**
  - Add the routing logic for `tachyon_deploy_function` (including reading the local artifact).
  - Add `tachyon_list_functions`, `tachyon_delete_function`, and `tachyon_function_logs`.

- [x] **Task 3: Implement KV & Traffic Handlers**
  - Add routing logic for the 3 KV operations mapping to `tachyon_client::kv_*`.
  - Add routing logic for `tachyon_canary_split` mapping to `tachyon_client::set_route_weights`.

- [x] **Task 4: E2E Testing**
  - Start the updated MCP server.
  - Ask an LLM (e.g., Claude Desktop) to: "Read the `examples/guest-example/target/wasm32-wasip2/release/guest_example.wasm` file and deploy it as 'test-faas'. Then, invoke it and check its logs using the KV store if necessary."