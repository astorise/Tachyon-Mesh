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

