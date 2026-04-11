## ADDED Requirements

### Requirement: The MCP wrapper reserves stdout for JSON-RPC payloads
The `tachyon-mcp` binary SHALL keep `stdout` clean for protocol traffic and SHALL route diagnostics exclusively to `stderr`.

#### Scenario: Runtime diagnostics do not corrupt the JSON-RPC stream
- **WHEN** the MCP server encounters an internal error while handling a request
- **THEN** the JSON-RPC error response is emitted on `stdout`
- **AND** any human-readable diagnostics are emitted on `stderr`
- **AND** the server does not write debug-only `println!` output to `stdout`
