## ADDED Requirements

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
