## ADDED Requirements

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

### Requirement: Tauri command and Workloads UI for package import
The system SHALL provide a `import_faas_package(bytes: Vec<u8>)` Tauri command
and an *Import & Deploy* section in `TachyonWorkloadsPanel` that lets the
operator select a `.tar.gz` file and trigger the import.

#### Scenario: Operator imports a package via the UI
- **WHEN** the operator selects a `.tar.gz` file and clicks *Deploy*
- **THEN** the file bytes are passed to the `import_faas_package` Tauri command
- **THEN** success or error feedback is displayed in the panel

### Requirement: Guest examples manifest shipped in CI artifact
The system SHALL include `examples/guest-examples/manifest.json` in the
`guest-examples.tar.gz` CI artifact, declaring routes for all non-test guest
WASMs present in the archive.

#### Scenario: Import of guest-examples artifact activates all routes
- **WHEN** an operator imports the `guest-examples.tar.gz` artifact
- **THEN** routes for guest-ai, guest-call-legacy, guest-example, guest-grpc,
  guest-log-storm, guest-loop, guest-voip-gate, guest-volume, and
  guest-websocket-echo are activated on the node
