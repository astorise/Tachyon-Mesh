# Design: Manifest JSON Schema Alignment

## Approach

Three coordinated changes across `core-host`, `system-faas-config-api`, and the MCP/client layer. Task 4 (cross-layer verification) is a manual QA step with no code artefact.

### 1. Schema endpoint (`core-host`)

`GET /admin/schema/manifest` is added to the admin router alongside the existing `/admin/metrics` and `/admin/canary` routes. The handler returns a hand-crafted JSON Schema (Draft-07) document using `serde_json::json!()`. This approach was chosen over `schemars` derive:

- Adding `#[derive(JsonSchema)]` to `IntegrityConfig` and all 25+ composite types spread across `domain_types.rs` and `runtime_types.rs` would be a purely mechanical change with no real benefit over a carefully authored schema.
- The hand-crafted schema is complete for the fields agents care about (all required fields, typed optional fields, nested `resiliency`, `resourcePolicy`, and `canary` blocks).
- The endpoint is unauthenticated-read (same tier as `/admin/status`) since schema discovery is not sensitive.

`ADMIN_SCHEMA_MANIFEST_PATH = "/admin/schema/manifest"` is added to `tachyon-client` so the cross-layer validation CI script catches route regressions.

### 2. Structured validation types (`system-faas-config-api`)

`ValidationError` and `DryRunResult` are added to `system-faas-config-api/src/lib.rs`. `serde` is added as a dependency (previously only `serde_json` was present).

`validate_manifest_payload(payload: &Value) -> DryRunResult` implements structural validation:
- Required top-level string fields (`hostAddress`, `resourceLimitResponse`)
- Required integer fields (`maxStdoutBytes`, `guestFuelBudget`, `guestMemoryLimitBytes`)
- `routes` must be an array; each entry must have string `path` and `version`, and `maxConcurrency` must be positive if present

Semantic validation (signature verification, SemVer ordering, route name resolution) remains the responsibility of `core-host`. The WASM component validates type correctness and required-field presence, producing machine-readable `ValidationError` values with JSONPointer paths.

### 3. MCP schema injection (`tachyon-mcp` + `tachyon-client`)

`tachyon_client::get_manifest_schema()` calls `GET /admin/schema/manifest`. A module-level `static MANIFEST_SCHEMA: OnceLock<Value>` is added to the MCP server.

In `validate_request_auth`, immediately after `CONNECTION_INITIALIZED` is set (connection established), `get_manifest_schema()` is called and the result is stored in `MANIFEST_SCHEMA`. The fetch is best-effort: failure is non-fatal (the tool still works with a generic schema fallback).

The `tachyon_dryrun_manifest` tool definition in `tools/list` reads `MANIFEST_SCHEMA.get()` and injects the rich schema into the `manifest` property, falling back to a minimal `"type": "object"` if the schema has not been fetched yet.

## Trade-offs

| Decision | Chosen | Rejected | Reason |
|---|---|---|---|
| Schema generation | Hand-crafted `serde_json::json!()` | `schemars` derive on 25+ structs | Avoids mechanical churn across two large files; hand-crafted schema can be authored once and evolves alongside the struct |
| Schema auth level | Unauthenticated read | Admin-token required | Schema is not sensitive; forcing auth would prevent tooling discovery |
| WASM validation scope | Structural only (type + required-field) | Full semantic validation | Semantic validation (signature, semver) requires host context not available in the WASM sandbox |
| MCP schema fetch timing | After first connection init | At MCP startup | Connection is not available at startup; first-request lazy init is already the established pattern |
