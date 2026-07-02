# workload-orchestration Specification

## Purpose
TBD - created by archiving change workload-orchestration-and-secrets. Update Purpose after archive.
## Requirements
### Requirement: Workload configurations MUST NOT contain plaintext secrets
The control plane SHALL enforce that any sensitive configuration passed to a workload is done via a `secret_ref`. The `system-faas-tde` SHALL intercept the execution start, decrypt the secret in memory, and inject it securely into the guest's environment.

#### Scenario: Running a workload with a decrypted secret
- **GIVEN** a `workload-spec` with a `secret-mount` referencing a valid TDE key
- **WHEN** the `core-host` instantiates the `faas_wasm` module
- **THEN** it resolves the secret locally via the TDE module
- **AND** injects it as an environment variable directly into the WASI context, ensuring it never touches the disk or the GitOps repository.

### Requirement: The Mesh MUST seamlessly route to diverse runtimes
The runtime orchestration SHALL support multiple execution backends (FaaS Wasm, SmolVM, Legacy Containers) under a unified configuration schema.

#### Scenario: Routing to a legacy container
- **GIVEN** a `workload-spec` configured with `runtime: legacy_container` and `endpoint: 127.0.0.1:8080`
- **WHEN** a client request is routed to this workload
- **THEN** the `core-host` bypasses the Wasm engine and acts as a high-performance Layer 4/7 reverse proxy forwarding the traffic to the specified endpoint.

### Requirement: Workload canary configuration is manifest-backed
The Tachyon UI workload panel SHALL configure canary rollouts by mutating the selected route's `canary` field in the active manifest. The panel SHALL NOT stage canary form data under `ui_configurations`.

#### Scenario: Operator configures canary rollout for a selected route
- **WHEN** the operator selects the Canary deployment strategy
- **AND** chooses a route from the manifest route selector
- **AND** enters `next_version`, `step_weight`, `interval_secs`, and `max_error_rate`
- **THEN** the panel reads the active manifest through `get_manifest_config`
- **AND** writes those values to the selected route's `routes[].canary`
- **AND** applies the updated manifest through `apply_manifest_config`

#### Scenario: Workload form does not fake unsupported runtime fields
- **WHEN** the operator submits Rolling strategy, engine, or secret fields that do not map to a current `IntegrityConfig` field
- **THEN** the UI displays a handled message explaining that the field is not manifest-backed yet
- **AND** no `workloads` payload is staged under `ui_configurations`
