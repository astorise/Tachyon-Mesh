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

