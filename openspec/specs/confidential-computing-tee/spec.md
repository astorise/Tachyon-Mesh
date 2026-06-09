# confidential-computing-tee Specification

## Purpose
TBD - created by archiving change opt-in-confidential-computing-tee. Update Purpose after archive.
## Requirements
### Requirement: integrity.lock allows flagging FaaS modules with requires_tee
The `integrity.lock` manifest SHALL accept a per-module `requires_tee: true` flag indicating that the corresponding Wasm module must execute inside a hardware Trusted Execution Environment (TEE).

#### Scenario: Manifest flags a module for TEE execution
- **WHEN** the manifest entry for a module includes `requires_tee: true`
- **THEN** the host treats the module as TEE-only
- **AND** rejects the configuration if no TEE backend is available on the node and the module is enabled

### Requirement: Core host delegates TEE-flagged modules to a hardware enclave backend
For modules flagged with `requires_tee: true`, the `core-host` SHALL bypass the standard pooled Wasmtime engine and delegate execution to a TEE-compatible backend (e.g. Enarx, WasmEdge SGX, or AWS Nitro Enclaves) where code and data live in hardware-encrypted memory.

#### Scenario: TEE-flagged module runs in a hardware enclave
- **WHEN** an incoming request targets a module flagged `requires_tee: true`
- **THEN** the host dispatches the request to the configured TEE backend rather than the standard pooled engine
- **AND** the module executes inside an attested enclave
- **AND** a host-level memory dump (e.g. by a privileged operator) reveals only encrypted bytes for that module's address range

### Requirement: Non-TEE traffic incurs no overhead from the TEE feature
Modules that do not set `requires_tee: true` SHALL continue to run on the standard pooled Wasmtime engine with no measurable latency overhead introduced by the TEE feature.

#### Scenario: Standard module is unaffected
- **WHEN** a module without the `requires_tee` flag is invoked
- **THEN** the host serves it from the pooled Wasmtime engine
- **AND** invocation latency matches the baseline measured before the TEE feature was introduced

### Requirement: TEE properties MUST be driven by the Control Plane
The `system-faas-tee-runtime` SHALL use the declarative configuration to determine which hardware attestation provider to use and whether to enforce strict TEE constraints.

#### Scenario: Strict enforcement of Confidential Computing
- **GIVEN** a node configuration with `strict_enforcement: true` for the TEE
- **WHEN** the node attempts to start on hardware that does not support the requested `tee_provider` (e.g., AMD SEV)
- **THEN** the `core-host` gracefully logs a fatal capability mismatch and halts
- **AND** refuses to load any sensitive WASM payloads into unsecured memory.

### Requirement: TEE delegation selects the configured backend or fails closed
When a route is flagged `requires_tee: true`, `core-host` SHALL dispatch the request to the backend named by the sealed `tee_backend` configuration. If no `tee_backend` is configured, the host SHALL fail closed with HTTP `503 Service Unavailable` rather than silently running the module on the pooled engine. Two backends SHALL be supported:

- `LocalEnclave` — runs the guest on the host's standard engine and annotates the response as TEE-served. This mode provides **no hardware memory encryption** and is intended for nodes without enclave hardware (development and local testing); the hardware-confidentiality guarantee of "Core host delegates TEE-flagged modules to a hardware enclave backend" applies only to the Enarx backend.
- `Enarx { keep_endpoint }` — delegates execution to an external Enarx Keep, gated behind the `enarx` Cargo feature and SGX/SEV-SNP hardware.

#### Scenario: Missing backend fails closed
- **WHEN** a request targets a `requires_tee` route and no `tee_backend` is configured
- **THEN** the host returns `503 Service Unavailable`
- **AND** the module is not executed on the pooled engine

#### Scenario: Local backend runs on the standard engine
- **WHEN** `tee_backend` is `LocalEnclave` and a `requires_tee` route is invoked
- **THEN** the host executes the guest on its standard engine off the async runtime
- **AND** annotates the response as served by the `local-enclave` backend

### Requirement: Enarx backend executes guests over a Keep endpoint
For the `Enarx` backend, the host SHALL POST a JSON invocation — module, route path, method, URI, headers, body, trailers, and trace id — to the configured `keep_endpoint`, and SHALL reconstruct the guest response from the Keep's JSON reply (status, headers, body, trailers, fuel consumed). A non-success HTTP status from the Keep, or an undecodable reply, SHALL surface as an internal execution error rather than a partial response.

#### Scenario: Keep round-trip produces the guest response
- **WHEN** the Enarx backend is invoked for a `requires_tee` route
- **THEN** the host POSTs the encoded invocation to `keep_endpoint`
- **AND** maps the Keep's structured reply into the guest HTTP response (status, headers, body, trailers) and fuel accounting

#### Scenario: Keep failure is not masked
- **WHEN** the Keep endpoint returns a non-2xx status or an undecodable body
- **THEN** the host raises an internal execution error
- **AND** does not return a partially populated response to the client

### Requirement: TEE-delegated responses are annotated with runtime headers
Every response produced through a TEE backend SHALL carry `x-tachyon-runtime: tee-{backend}` and `x-tachyon-tee-backend: {backend}` (`local-enclave` or `enarx`), replacing any same-named headers supplied by the guest or upstream Keep so the runtime annotation is authoritative.

#### Scenario: Backend identity is stamped on the response
- **WHEN** a `requires_tee` route is served through any TEE backend
- **THEN** the response carries `x-tachyon-tee-backend` naming the backend
- **AND** carries `x-tachyon-runtime` set to `tee-{backend}`
- **AND** any client- or Keep-supplied values for those two headers are discarded

