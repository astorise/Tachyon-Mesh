# mcp-server Specification

## Purpose
TBD - created by archiving change unified-client-triad. Update Purpose after archive.
## Requirements
### Requirement: A shared local client library serves both desktop and MCP wrappers
The workspace SHALL provide a `tachyon-client` Rust library crate with async helpers for reading `integrity.lock` and computing an engine status payload for local control-plane clients.

#### Scenario: Shared client reads the lockfile asynchronously
- **WHEN** a local wrapper calls `tachyon_client::read_lockfile()`
- **THEN** the function reads `integrity.lock` from the workspace root asynchronously
- **AND** the function returns the raw lockfile payload as a UTF-8 string

### Requirement: The MCP wrapper exposes shared status tools over JSON-RPC
The workspace SHALL provide a `tachyon-mcp` binary that speaks JSON-RPC 2.0 over `stdin` / `stdout` and delegates tool execution to `tachyon-client`.

#### Scenario: The AI requests Tachyon mesh status
- **WHEN** the server receives a `tools/call` request for `tachyon_mesh_status`
- **THEN** it awaits `tachyon_client::get_engine_status()`
- **AND** it returns the shared client response in the JSON-RPC result payload

#### Scenario: The AI requests the current lockfile
- **WHEN** the server receives a `tools/call` request for `tachyon_lockfile`
- **THEN** it awaits `tachyon_client::read_lockfile()`
- **AND** it writes only JSON-RPC responses to `stdout`
- **AND** any diagnostic logging is written to `stderr`

### Requirement: The MCP wrapper reserves stdout for JSON-RPC payloads
The `tachyon-mcp` binary SHALL keep `stdout` clean for protocol traffic and SHALL route diagnostics exclusively to `stderr`.

#### Scenario: Runtime diagnostics do not corrupt the JSON-RPC stream
- **WHEN** the MCP server encounters an internal error while handling a request
- **THEN** the JSON-RPC error response is emitted on `stdout`
- **AND** any human-readable diagnostics are emitted on `stderr`
- **AND** the server does not write debug-only `println!` output to `stdout`

### Requirement: The MCP server exposes a list_resources tool
The `tachyon-mcp` binary SHALL register a `tachyon_list_resources` JSON-RPC tool whose handler delegates to `tachyon_client::read_resources()` and returns the merged list of sealed and pending mesh resources as a JSON array in the tool result content.

#### Scenario: An AI agent enumerates configured resources
- **WHEN** the MCP server receives a `tools/call` request for `tachyon_list_resources`
- **THEN** it awaits `tachyon_client::read_resources()`
- **AND** it returns a JSON array containing every sealed and overlay resource
- **AND** overlay entries include a `pending: true` field so agents can detect they require a CLI re-seal

### Requirement: The MCP server exposes a register_resource tool
The `tachyon-mcp` binary SHALL register a `tachyon_register_resource` JSON-RPC tool that accepts a JSON object matching the mesh-resource schema (`name`, `type`, `target`, plus type-specific fields), validates the input through the same helper used by the desktop `save_resource` Tauri command, and writes the entry to the workspace overlay file `tachyon.resources.json`.

#### Scenario: An AI agent registers a new external resource
- **WHEN** the MCP server receives a `tools/call` for `tachyon_register_resource` with `{ "name": "github-api", "type": "external", "target": "https://api.github.com", "allowed_methods": ["GET"] }`
- **THEN** the server validates the HTTPS target through the shared validator
- **AND** it persists the entry via `tachyon_client::upsert_overlay_resource`
- **AND** it returns a success result that mentions the resource is pending CLI re-seal

#### Scenario: Invalid registration is rejected without writing the overlay
- **WHEN** the MCP server receives a `tools/call` for `tachyon_register_resource` with an empty `name` or a non-HTTPS `target`
- **THEN** the server returns a JSON-RPC error describing the violated rule
- **AND** the overlay file `tachyon.resources.json` is left unchanged

### Requirement: MCP validates configured PATs for every JSON-RPC request
The `tachyon-mcp` binary SHALL require both `TACHYON_MCP_URL` and a PAT before accepting non-initialization requests, and SHALL validate the PAT against the configured host per request.

#### Scenario: MCP handles a tool call
- **WHEN** the server receives a JSON-RPC request after initialization
- **THEN** it verifies the configured PAT against `TACHYON_MCP_URL`
- **AND** expired, missing, or rejected tokens produce a JSON-RPC error instead of allowing tool execution

### Requirement: MCP applies per-tool rate limits
The `tachyon-mcp` binary SHALL rate-limit write-heavy tools independently from read-oriented tools and SHALL persist short-lived bucket state under the system temporary directory.

#### Scenario: Heavy manifest apply exceeds its bucket
- **WHEN** `tachyon_apply_manifest` is called more than once in its one-minute bucket
- **THEN** the server returns a JSON-RPC rate-limit error
- **AND** calls to read-oriented tools use independent buckets

#### Scenario: Rate limiter lock is poisoned
- **WHEN** the rate limiter mutex cannot be acquired cleanly
- **THEN** the server returns a structured JSON-RPC internal error
- **AND** it does not panic or terminate the process

### Requirement: Bounded tool contract — no unimplemented streaming
The `tachyon_tail_logs` tool schema MUST NOT advertise a `follow` parameter that is not implemented.

#### Scenario: Agent calls tachyon_tail_logs
- **WHEN** an agent invokes `tachyon_tail_logs` with or without a `lines` argument
- **THEN** the server returns a fixed snapshot of the last N log lines
- **AND** the response contains no `followRequested` field

### Requirement: Non-blocking hardware status
The `resources/read` hardware resource handler and the `tachyon_hardware_status` tool MUST offload the synchronous sysinfo call to a Tokio blocking thread to avoid stalling the async executor.

#### Scenario: Hardware status is requested under load
- **GIVEN** the MCP server is handling requests
- **WHEN** `hardware://local/status` or `tachyon_hardware_status` is called
- **THEN** `read_local_hardware_status` runs on the Tokio blocking thread pool

### Requirement: Connection initialized once per process
The PAT validation against `core-host` SHALL happen at most once per MCP server process lifetime.

#### Scenario: Agent sends multiple tool calls in a session
- **WHEN** the agent sends multiple consecutive requests
- **THEN** `set_connection` is called exactly once
- **AND** subsequent requests skip the HTTP round-trip and reuse the cached state

### Requirement: Dynamic manifest schema injection
The `tachyon_dryrun_manifest` tool definition SHALL include the full `IntegrityConfig` JSON Schema in its `inputSchema.properties.manifest` field, fetched from `GET /admin/schema/manifest`.

#### Scenario: Schema is available after first authenticated request
- **GIVEN** the MCP server has completed its initial `set_connection` call
- **WHEN** an agent sends `tools/list`
- **THEN** the `tachyon_dryrun_manifest` tool's `inputSchema` contains the full IntegrityConfig schema as the `manifest` property

#### Scenario: Schema is unavailable at startup
- **GIVEN** the MCP server has not yet authenticated (no first request)
- **WHEN** `tools/list` is called
- **THEN** `manifest` falls back to a generic `{"type": "object"}` schema

### Requirement: Structured JSON-RPC error taxonomy
The MCP server SHALL return typed JSON-RPC errors with machine-readable codes and structured `data` fields instead of a flat string message.

#### Scenario: Tool call times out
- **GIVEN** `TACHYON_MCP_TIMEOUT_MS` is set (or defaults to 5 000 ms)
- **WHEN** a tachyon_client call exceeds the deadline
- **THEN** the response error code is `-32001` with `message` referencing the timeout duration

#### Scenario: Rate limit exceeded
- **GIVEN** a tool's per-minute bucket is exhausted
- **WHEN** another call arrives for that tool
- **THEN** the response error code is `-32002`
- **AND** `error.data.retry_after_ms` contains the milliseconds until the window resets

#### Scenario: Invalid manifest payload
- **GIVEN** a tachyon_dryrun_manifest call fails structural validation
- **WHEN** `JsonRpcError::from_anyhow` classifies the error
- **THEN** the response error code is `-32602`
- **AND** `error.data.detail` carries the validation message

#### Scenario: Unexpected internal failure
- **GIVEN** a tool call fails for an unclassified reason
- **WHEN** `JsonRpcError::from_anyhow` classifies the error
- **THEN** the response error code is `-32603`

### Requirement: Advanced MCP tools — WASM lifecycle and KV operations
The MCP server SHALL expose tools for deploying and managing WASM functions, reading/writing the KV-Partition V2 store, and adjusting canary traffic splits.

#### Scenario: Agent deploys a WASM artifact
- **WHEN** `tachyon_deploy_function` is called with `function_name` and `artifact_path`
- **THEN** the artifact is read from disk and uploaded as a named mesh asset
- **AND** a workload configuration overlay is staged
- **AND** the response advises the agent to run `tachyon_seal_overlay`

#### Scenario: Agent reads a KV-Partition value
- **WHEN** `tachyon_kv_get` is called with `namespace` and `key`
- **THEN** the UTF-8 string value is returned, or `(key not found)` if absent

#### Scenario: Agent adjusts canary traffic split
- **GIVEN** an active canary rollout exists for `route_path`
- **WHEN** `tachyon_canary_split` is called with `weight_pct > 0`
- **THEN** the live rollout weight is updated to the specified percentage via `PATCH /admin/canary`
- **WHEN** `tachyon_canary_split` is called with `weight_pct = 0`
- **THEN** the rollout is aborted and traffic reverts to the stable version

### Requirement: tachyon-mcp MUST have a stdio E2E test harness
A Rust integration test at `tachyon-mcp/tests/mcp_e2e_runner.rs` SHALL spawn the compiled `tachyon-mcp` binary, drive it via stdin/stdout, and assert that: (1) `initialize` returns the correct MCP protocol version; (2) `tools/list` returns a structurally valid JSON-RPC response; (3) with a live cluster, the core tools are present and read-only calls do not return `-32603`.

#### Scenario: initialize returns protocol version without a cluster
- **GIVEN** the binary is spawned with an unreachable cluster URL
- **WHEN** `{"jsonrpc":"2.0","id":1,"method":"initialize"}` is sent on stdin
- **THEN** stdout contains a JSON-RPC response with `result.protocolVersion = "2025-03-26"`

#### Scenario: tools/list is structurally valid even when the cluster is unreachable
- **GIVEN** no live cluster is available
- **WHEN** `tools/list` is sent after `initialize`
- **THEN** the response contains either `result.tools` (array) or `error.code` in `[-32001, -32002, -32600, -32602, -32603]`
- **AND** the response is never malformed JSON

#### Scenario: Live-cluster test asserts critical tool presence
- **GIVEN** `E2E_CLUSTER_URL` and `E2E_CLUSTER_PAT` are set
- **WHEN** `tools/list` is called
- **THEN** `tachyon_hardware_status`, `tachyon_topology_snapshot`, and `tachyon_dryrun_manifest` are in the tools array
- **AND** `tachyon_dryrun_manifest.inputSchema.properties` is a non-empty object (dynamic manifest schema injected)

### Requirement: tachyon_hardware_status MUST include GPU topology in its response
The `tachyon_hardware_status` MCP tool SHALL return a JSON payload that includes a `gpus` array. Each entry SHALL carry `id`, `model`, `vramTotalMb`, `vramUsedMb`, and `computeUtilization`. When no GPU management library is linked, VRAM values SHALL default to 0 rather than being omitted.

#### Scenario: Response includes gpus array
- **GIVEN** the cluster node has `CUDA_VISIBLE_DEVICES=0` set
- **WHEN** an agent calls `tachyon_hardware_status`
- **THEN** the response JSON contains a non-empty `gpus` array with an entry for `id: "cuda:0"`
- **AND** the entry has `vramTotalMb` and `vramUsedMb` fields (may be 0)

#### Scenario: Response includes gpus array even without GPU
- **GIVEN** no CUDA or HIP environment variables are set
- **WHEN** an agent calls `tachyon_hardware_status`
- **THEN** the response JSON contains `"gpus": []`
- **AND** `accelerators` contains only `"cpu"`

### Requirement: Mutator tools MUST have stricter rate limits than read-only tools
The MCP server SHALL apply per-tool rate limits: `tachyon_canary_split` ≤ 2/min, `tachyon_deploy_function`/`tachyon_delete_function` ≤ 5/min, KV mutators ≤ 30/min, read-only tools ≤ 100/min. Exceeding any limit returns `-32002` with `retry_after_ms`.

#### Scenario: Canary split is rate-limited to 2/min
- **GIVEN** an agent has already called `tachyon_canary_split` twice in one minute
- **WHEN** it calls a third time
- **THEN** the response contains `error.code = -32002` and `error.data.retry_after_ms`

### Requirement: Mutator tool descriptions MUST include explicit LLM guidance
`tachyon_deploy_function` description SHALL state that `artifact_path` must be an absolute local path on the MCP host. `tachyon_kv_put` description SHALL mandate JSON-stringified values. `tachyon_canary_split` description SHALL explain that `weight_pct=0` performs an immediate rollback.

### Requirement: The legacy error_response() function MUST be removed
The `error_response(id, code, message)` function SHALL be deleted in favour of `json_rpc_error_response(id, &JsonRpcError)`, which produces a fully structured error object consistent with all other error paths.

### Requirement: Schema fetch failure MUST emit a tracing warning and populate tools/list warnings
When `get_manifest_schema()` fails, `tachyon-mcp` SHALL emit a `tracing::warn!` describing the degradation. The `tools/list` response SHALL include `data.warnings` when `MANIFEST_SCHEMA` is unpopulated.
