## ADDED Requirements

### Requirement: Privileged bridge controllers can request zero-WASM Layer 4 relays for active media sessions
The platform SHALL allow a privileged bridge controller to allocate dynamic Layer 4 bridge endpoints that the host relays directly without invoking WASM for every packet.

#### Scenario: A media bridge is created for an active session
- **WHEN** a privileged bridge controller requests a new bridge between live endpoints
- **THEN** the host allocates the required ports and relay tasks
- **AND** forwards traffic directly between those endpoints without per-packet guest execution

### Requirement: User guests allocate bridges through the sealed system bridge route
The platform SHALL expose a typed bridge controller API to user guests and route their bridge requests through `/system/bridge`.

#### Scenario: A user guest starts a call
- **WHEN** a user guest calls `bridge-controller.create-bridge`
- **THEN** the host forwards the request to the privileged `/system/bridge` route
- **AND** returns the bridge identifier and allocated ports to the caller
