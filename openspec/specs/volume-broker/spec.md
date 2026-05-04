# volume-broker Specification

## Purpose
TBD - created by archiving change volume-broker. Update Purpose after archive.
## Requirements
### Requirement: User targets cannot request writable direct host mounts
The integrity manifest SHALL reject any configuration where a `role: "user"` target requests a writable host volume mount.

#### Scenario: A user target asks for a writable mount
- **WHEN** a user target declares a mount with writable access to a host-backed volume
- **THEN** manifest validation fails before the host starts

### Requirement: User writes are delegated through a storage broker system target
The platform SHALL expose a storage broker system FaaS that accepts authenticated write requests for shared storage instead of allowing direct writable mounts to user targets.

#### Scenario: A user target submits a storage write
- **WHEN** a user target needs to write to a shared volume path
- **THEN** it sends the payload to the storage broker over the internal mesh
- **AND** the broker receives the desired path and write mode as request metadata

### Requirement: The storage broker serializes writes through an internal queue
The storage broker SHALL acknowledge accepted write requests quickly and process them sequentially through an internal queue to preserve consistency on the underlying filesystem.

#### Scenario: The broker accepts concurrent write requests
- **WHEN** multiple write requests arrive at the broker concurrently
- **THEN** the broker enqueues each request
- **AND** returns HTTP 202 without waiting for the file write to finish
- **AND** executes queued writes one at a time against the filesystem

### Requirement: WASI Preopens MUST be declaratively mapped
The Volume Broker SHALL mount physical host directories or memory partitions into WebAssembly guests dynamically, based on the declarative `StorageConfiguration` schema.

#### Scenario: Provisioning a temporary memory filesystem
- **WHEN** the `system-faas-config-api` receives a valid `wasi-volume` with `type: memory_tmpfs`
- **THEN** the host provisions a virtual tmpfs capped to `max_size_mb`
- **AND** preopens this directory into the guest's WASI environment at the specified `guest_path` without requiring host filesystem access.

