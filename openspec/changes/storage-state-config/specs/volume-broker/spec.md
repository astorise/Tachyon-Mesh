# volume-broker Delta

## ADDED Requirements

### Requirement: WASI Preopens MUST be declaratively mapped
The Volume Broker SHALL mount physical host directories or memory partitions into WebAssembly guests dynamically, based on the declarative `StorageConfiguration` schema.

#### Scenario: Provisioning a temporary memory filesystem
- **WHEN** the `system-faas-config-api` receives a valid `wasi-volume` with `type: memory_tmpfs`
- **THEN** the host provisions a virtual tmpfs capped to `max_size_mb`
- **AND** preopens this directory into the guest's WASI environment at the specified `guest_path` without requiring host filesystem access.
