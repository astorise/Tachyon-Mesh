# confidential-computing-tee Delta

## ADDED Requirements

### Requirement: TEE properties MUST be driven by the Control Plane
The `system-faas-tee-runtime` SHALL use the declarative configuration to determine which hardware attestation provider to use and whether to enforce strict TEE constraints.

#### Scenario: Strict enforcement of Confidential Computing
- **GIVEN** a node configuration with `strict_enforcement: true` for the TEE
- **WHEN** the node attempts to start on hardware that does not support the requested `tee_provider` (e.g., AMD SEV)
- **THEN** the `core-host` gracefully logs a fatal capability mismatch and halts
- **AND** refuses to load any sensitive WASM payloads into unsecured memory.
