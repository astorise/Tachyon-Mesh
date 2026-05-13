# Design: Advanced MCP Tools

## Approach

Three coordinated layers: admin endpoints in `core-host`, client functions in `tachyon-client`, and MCP tool definitions + handlers in `tachyon-mcp`.

### 1. Core-host admin endpoints

**KV-Partition V2 (`/admin/kv/{namespace}/{key}`)**  
Three REST handlers use `AppState.core_store.kv_partition_{get,set,delete}` from the existing `store/mod.rs` implementation. The route uses Axum path extractors for the two-segment key.

**Canary weight override (`PATCH /admin/canary`)**  
A new `admin_set_canary_weight_handler` allows operators (and agents) to manually override the traffic-split percentage of an in-flight canary rollout without aborting the evaluator. Setting `weight_pct = 100` transitions the phase to `Promoted`. The existing `POST /admin/canary` abort path is unchanged.

### 2. tachyon-client additions

| Function | Mapping |
|---|---|
| `kv_get(ns, key)` | `GET /admin/kv/{ns}/{key}` → `Option<Vec<u8>>` |
| `kv_put(ns, key, bytes)` | `PUT /admin/kv/{ns}/{key}` with raw body |
| `kv_delete(ns, key)` | `DELETE /admin/kv/{ns}/{key}` |
| `list_functions()` | `get_mesh_graph()` → extracts routes |
| `deploy_function(name, bytes, ram, vram)` | `push_asset_bytes` + `stage_configuration_overlay("workloads", …)` |
| `delete_function(name)` | `remove_overlay_resource(name)` |
| `function_logs(name, n)` | `tail_logs` with `target` query param, filtered in-process |
| `set_canary_split(path, weight)` | `PATCH /admin/canary` (weight > 0) or `POST /admin/canary` (weight = 0 → abort) |

### 3. MCP tools

**WASM lifecycle** (`tachyon_deploy_function`, `tachyon_list_functions`, `tachyon_delete_function`, `tachyon_function_logs`): the deploy handler reads the artifact from disk with `tokio::fs::read`, so it works when the MCP server runs locally alongside the agent filesystem. Deployment is two-phase: upload + stage, then explicit `tachyon_seal_overlay`.

**KV-Partition** (`tachyon_kv_get`, `tachyon_kv_put`, `tachyon_kv_delete`): the value field is a UTF-8 string in the MCP schema; bytes are passed as `value.as_bytes()` to allow JSON-stringified structured data.

**Canary** (`tachyon_canary_split`): `weight_pct = 0` aborts; any other value patches the live rollout weight. Agents can use this for fine-grained traffic control without re-sealing the manifest.

All 8 new handlers go through `handle_tool_dispatch` which is wrapped in `tokio::time::timeout(mcp_timeout())` from the previous error-taxonomy change.

## Trade-offs

| Decision | Chosen | Rejected | Reason |
|---|---|---|---|
| Deploy two-phase | Upload + stage overlay | Direct hot-patch | Preserves integrity guarantees; agent must explicitly seal |
| KV value type | UTF-8 string in MCP schema | Base64 bytes | Agents emit readable JSON values; base64 adds unnecessary friction |
| `function_logs` filter | In-process after `tail_logs` | Dedicated route-level log endpoint | Avoids new admin route; `LogLine.target` already carries the route path |
| Canary weight via PATCH | New `PATCH /admin/canary` | Reuse POST (abort) endpoint | Semantic separation: abort vs update; PATCH is idempotent and safe |
