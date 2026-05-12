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
