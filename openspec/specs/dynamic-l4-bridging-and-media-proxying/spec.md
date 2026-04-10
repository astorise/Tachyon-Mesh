# Dynamic L4 Bridging And Media Proxying

## Purpose
Define how Tachyon allocates native UDP relay bridges for media traffic while keeping bridge orchestration in sealed WASM components.

## Requirements
### Requirement: The host can allocate zero-WASM UDP relay bridges
The platform SHALL allow the host to allocate paired UDP relay ports and forward datagrams directly between remote media endpoints without invoking WASM for each packet.

#### Scenario: A bridge is allocated for two live endpoints
- **WHEN** the host receives a valid bridge allocation request
- **THEN** it binds two ephemeral UDP sockets
- **AND** relays datagrams between the configured remote endpoints in a dedicated native task

#### Scenario: A bridge becomes idle
- **WHEN** no datagrams traverse a bridge within the configured timeout
- **THEN** the host tears down the relay
- **AND** releases the associated bridge ports

### Requirement: User guests allocate bridges through a privileged system controller
The platform SHALL expose a typed bridge controller API to user guests while routing actual bridge allocation through the sealed `/system/bridge` system route.

#### Scenario: A user guest requests a bridge
- **WHEN** a user guest calls `bridge-controller.create-bridge`
- **THEN** the host forwards the request to `/system/bridge`
- **AND** the system bridge route receives direct access to the shared bridge manager
- **AND** the caller receives the allocated bridge identifier and ports

### Requirement: The system bridge route persists bridge session metadata
The privileged system bridge component SHALL record bridge lifecycle metadata in its writable RAM volume so sessions can be monitored and torn down intentionally.

#### Scenario: A bridge is created through `/system/bridge`
- **WHEN** the system bridge route provisions a new bridge
- **THEN** it persists a session record in `/sessions`
- **AND** the record contains the bridge identifier, remote endpoints, timeout, and allocated ports
