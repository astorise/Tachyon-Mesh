# Implementation Tasks

- [x] **Task 1: Core-Host Schema Export**
  - Add `schemars` to `core-host`.
  - Derive `JsonSchema` on all Manifest structs.
  - Implement `GET /admin/schema/manifest`.

- [x] **Task 2: WASM Validation (System FaaS)**
  - Update `systems/system-faas-config-api` to include the `jsonschema` crate.
  - Implement the structured `ValidationError` and `DryRunResult` types.
  - Write the validation logic that compares incoming payloads against the schema and returns the structured result.

- [x] **Task 3: MCP Tool Update**
  - Modify `tachyon-mcp/src/main.rs`.
  - Fetch the schema on startup and inject it into the `tachyon_apply_manifest` and `tachyon_dryrun_manifest` tool definitions.

- [x] **Task 4: Cross-Layer Verification**
  - Deploy the updated `system-faas-config-api` to the local test cluster.
  - Trigger a dry-run via MCP with an intentionally broken manifest (e.g., passing a string for `minRamMb` instead of an integer).
  - Verify that the MCP agent receives a structured `-32602` or equivalent JSON response pointing exactly to `spec.functions[0].minRamMb`.