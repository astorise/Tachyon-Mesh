## ADDED Requirements

### Requirement: The MCP server exposes an upload_model tool
The `tachyon-mcp` binary SHALL register a `tachyon_upload_model` JSON-RPC tool that accepts a required string `path` argument — an absolute local path to a model directory (weights plus `tokenizer.json`, and `config.json` for safetensors) or a single self-contained file on the MCP host — and delegates to `tachyon_client::push_large_model(path)`, returning the resulting server-side model path in the tool result content. The tool's `inputSchema` SHALL declare `required: ["path"]`, the missing-`path` case SHALL be rejected before any cluster call, and the tool SHALL be governed by the same tight per-minute rate-limit budget as other large, hash-verified mutators.

#### Scenario: Upload delegates to the model broker
- **WHEN** the MCP server receives a `tools/call` for `tachyon_upload_model` with a string `path`
- **THEN** it calls `tachyon_client::push_large_model(path)`
- **AND** returns the broker's server-side model path in the result content

#### Scenario: Missing path is rejected before dispatch
- **WHEN** a `tachyon_upload_model` call omits the `path` argument
- **THEN** the server returns an invalid-params error (`-32602`) without contacting the cluster

#### Scenario: Upload is rate-limited as a heavy mutator
- **WHEN** `tachyon_upload_model` is called more often than its per-minute budget allows
- **THEN** further calls return the rate-limited error (`-32002`) until the bucket refills
