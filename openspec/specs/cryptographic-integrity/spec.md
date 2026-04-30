# cryptographic-integrity Specification

## Purpose
Define how Tachyon Mesh proves that the sealed deployment configuration and cached Wasm artifacts match the code that is allowed to execute.

## Requirements
### Requirement: Signed integrity configuration
The core host SHALL verify `integrity.lock` with Ed25519 before accepting traffic.

#### Scenario: Invalid signature is rejected
- **WHEN** the embedded configuration payload does not match the configured public key and signature
- **THEN** startup fails before listeners are exposed

### Requirement: Cwasm cache soundness
The Cwasm cache SHALL bind precompiled artifacts to the Wasmtime engine compatibility hash.

#### Scenario: Engine compatibility changes
- **WHEN** the host starts with an engine hash that differs from the hash recorded for cached artifacts
- **THEN** the stale cache bucket is purged before cached components or modules are deserialized
