# audit-closure Specification

## Purpose
Close audit findings for Tachyon UI and MCP by tightening WIT contract alignment, MCP authentication, UI rendering safety, graceful startup failure handling, and localized backend errors.

## Requirements

### Requirement: UI WIT Contract Synchronization
Tachyon-UI SHALL depend on WIT contracts for configuration validation so local DTO drift is detected during development.

#### Scenario: UI backend builds with WIT contracts
- **GIVEN** the Tauri backend is compiled
- **WHEN** configuration validation code is built
- **THEN** WIT bindings are generated from the workspace `wit/` contracts
- **AND** stale hand-written configuration DTOs are not required for validation

### Requirement: MCP Authentication and Write Limits
The MCP server SHALL require a PAT before accepting JSON-RPC requests and SHALL rate-limit write tools.

#### Scenario: MCP starts without a PAT
- **GIVEN** no `--token` argument or `TACHYON_MCP_PAT` environment variable is provided
- **WHEN** the MCP server starts
- **THEN** it exits before processing JSON-RPC input

#### Scenario: Agent exceeds write budget
- **GIVEN** an authenticated MCP session has already consumed the write allowance
- **WHEN** a write tool is called again inside the same minute
- **THEN** the server returns JSON-RPC error `-32000`
- **AND** the error message says `Rate limit exceeded`

### Requirement: UI XSS Hardening and Graceful Exit
Tachyon-UI SHALL avoid injecting untrusted runtime values through `innerHTML` and SHALL exit gracefully on fatal Tauri startup errors.

#### Scenario: Runtime message is displayed
- **GIVEN** a runtime route or toast message includes user-controlled text
- **WHEN** the UI renders it
- **THEN** the value is assigned through `textContent` or DOM APIs

#### Scenario: Tauri startup fails
- **GIVEN** the Tauri application fails to run
- **WHEN** startup returns an error
- **THEN** the application logs a fatal error to stderr
- **AND** exits with status code 1

### Requirement: Backend Error Localization
Tachyon-UI SHALL translate known backend validation and security errors before displaying them to operators.

#### Scenario: Backend validation error reaches UI
- **GIVEN** a Rust backend error is returned from a Tauri command
- **WHEN** the frontend catches the failure
- **THEN** known error strings are mapped to localized operator-facing messages
- **AND** unknown errors receive a localized generic system error prefix
