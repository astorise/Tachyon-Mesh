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

