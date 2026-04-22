## ADDED Requirements

### Requirement: Admin storage flows are delegated to dedicated system FaaS components
The host SHALL stop implementing the asset registry and large-model broker directly in `core-host` and SHALL delegate those workflows to dedicated system FaaS components.

#### Scenario: Asset registry uploads are proxied through a system guest
- **WHEN** an operator uploads a `.wasm` asset through `/admin/assets`
- **THEN** the host forwards the request to `system-faas-registry`
- **AND** the system guest persists the asset under a stable content-addressed hash
- **AND** the response returns a `tachyon://sha256:...` URI

#### Scenario: Large model uploads follow a chunked disk-backed protocol
- **WHEN** an operator uploads a model through the init/upload/commit API
- **THEN** the host forwards the multipart flow to `system-faas-model-broker`
- **AND** each chunk is appended directly to a staging file on disk
- **AND** the commit step verifies the final checksum before promoting the file into the model store

### Requirement: The desktop UI exposes an initial connection overlay
The desktop UI SHALL render the `connection-overlay` form before the module script so the operator can configure the target node, token, and mTLS profile before interacting with the dashboard.

#### Scenario: The operator opens Tachyon UI for the first time
- **WHEN** `tachyon-ui/index.html` loads
- **THEN** the page contains the `connection-overlay` markup before the module script tag
- **AND** the client logic binds the connection workflow to the `connect-btn` action

### Requirement: HTTP/3 administrative routes are zero-trust gated
The HTTP/3 listener SHALL validate bearer tokens for `/admin/*` requests through the `system-faas-auth` component before returning administrative data.

#### Scenario: An unauthenticated HTTP/3 admin request arrives
- **WHEN** a request targets an administrative path without a valid bearer token
- **THEN** the HTTP/3 listener rejects it with `401 Unauthorized` or `403 Forbidden`
- **AND** the request is not dispatched to the administrative handler
