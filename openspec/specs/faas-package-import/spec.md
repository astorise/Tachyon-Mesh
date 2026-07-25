# faas-package-import Specification

## Purpose
TBD - created by archiving change import-faas-package. Update Purpose after archive.
## Requirements
### Requirement: Import FaaS package from archive
The system SHALL accept a `.tar.gz` archive containing `.wasm` files and a
`manifest.json`, upload each WASM as a content-addressed asset, and register the
routes defined in `manifest.json` on the connected live node in a single
operation.

#### Scenario: Successful package import
- **WHEN** `import_faas_package_bytes` is called with a valid archive while connected to a node
- **THEN** every `.wasm` file is uploaded via `push_asset_bytes` and receives a `tachyon://sha256:…` URI
- **THEN** routes in `manifest.json` have their `module` / `targets[].module` replaced with the corresponding asset URI
- **THEN** the live manifest is patched with the new routes and POSTed to the node
- **THEN** the function returns `ImportPackageResult` with `imported_modules`, `skipped_modules`, and `routes_added`

#### Scenario: Import fails when not connected
- **WHEN** `import_faas_package_bytes` is called without an active node connection
- **THEN** the function returns an error "not connected to a node" immediately, before reading any WASM data

#### Scenario: Routes with no matching WASM are skipped
- **WHEN** `manifest.json` references a module name not present in the archive
- **THEN** that route is listed in `skipped_modules` and excluded from the manifest patch
- **THEN** all other matching routes are still applied

#### Scenario: Module name forms resolved via underscore/dash normalisation
- **WHEN** a WASM stem uses underscores (e.g. `guest_call_legacy`) and the manifest references the dash form (`guest-call-legacy`)
- **THEN** the import resolves the match correctly and assigns the asset URI

### Requirement: MCP tool for package import
The system SHALL expose `tachyon_import_package` as an MCP tool accepting
`package_path` (string, required) and returning the `ImportPackageResult` as
pretty-printed JSON.

#### Scenario: MCP tool invocation
- **WHEN** an MCP client calls `tachyon_import_package` with a valid `package_path`
- **THEN** the tool delegates to `tachyon_client::import_faas_package` and returns `{ content: [{ type: "text", text: <json> }] }`

### Requirement: RAG vector example is importable as a FaaS package
The repository SHALL provide a package manifest for `examples/guest-rag-vector` that can be bundled with its compiled WASM artifact and imported through `tachyon_import_package`. The manifest SHALL declare `/api/guest-rag-vector` as a user route backed by the `guest-rag-vector` module, with explicit `vector` scope for its demo indexes and explicit `http` scope for the OpenAI-compatible embedding and chat completion routes.

#### Scenario: Operator imports the RAG vector example package
- **WHEN** an operator imports a package containing `guest-rag-vector.wasm` and `examples/guest-rag-vector/manifest.json`
- **THEN** the import maps the manifest's `guest-rag-vector` module reference to the uploaded WASM asset URI
- **AND** the live manifest gains the `/api/guest-rag-vector` route
- **AND** the route has `vector` and `http` scopes sufficient to run the documented RAG demo

#### Scenario: Imported RAG route is agent-queryable
- **GIVEN** `/api/guest-rag-vector` has been imported and applied
- **WHEN** an MCP agent calls `tachyon_vector_search` without specifying `route_path`
- **THEN** the MCP tool queries `/api/guest-rag-vector`
- **AND** returns the route's RAG response to the agent

### Requirement: Tauri command and Workloads UI for package import
The system SHALL provide a `import_faas_package(bytes: Vec<u8>)` Tauri command
and an *Import & Deploy* section in `TachyonWorkloadsPanel` that lets the
operator select a `.tar.gz` file and trigger the import.

#### Scenario: Operator imports a package via the UI
- **WHEN** the operator selects a `.tar.gz` file and clicks *Deploy*
- **THEN** the file bytes are passed to the `import_faas_package` Tauri command
- **THEN** success or error feedback is displayed in the panel

### Requirement: Guest examples manifest shipped in CI artifact

The `examples/guest-examples/manifest.json` shipped in the `guest-examples.tar.gz` CI artifact SHALL declare routes for all practical (non-test, HTTP/WS/gRPC) guest WASMs present in the archive: `guest-ai`, `guest-call-legacy`, `guest-example`, `guest-grpc` (as `/grpc/hello`), `guest-log-storm`, `guest-loop`, `guest-voip-gate`, `guest-volume`, and `guest-websocket-echo`. It SHALL ALSO declare the OpenAI-compatible example routes backed by the `guest-openai` module: `/ai/v1/models`, `/ai/v1/chat/completions`, `/ai/v1/embeddings`, and `/internal/guest-openai/register`. It SHALL NOT declare `/v1/models`, `/v1/chat/completions`, or `/v1/embeddings`. `guest-flaky`, `guest-malicious`, `guest-tcp-echo`, and `guest-udp-echo` SHALL be excluded.

#### Scenario: Import activates the guest and OpenAI example routes

- **WHEN** an operator imports the `guest-examples.tar.gz` artifact
- **THEN** 13 routes are added to the live manifest (routes_added = 13): the 9 practical guest routes plus the 4 `guest-openai` routes
- **AND** `/ai/v1/models`, `/ai/v1/chat/completions`, and `/ai/v1/embeddings` are active
- **AND** `/v1/models`, `/v1/chat/completions`, and `/v1/embeddings` are not active
- **AND** `guest-flaky`, `guest-malicious`, `guest-tcp-echo`, `guest-udp-echo` are NOT activated

