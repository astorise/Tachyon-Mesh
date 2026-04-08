## ADDED Requirements

### Requirement: Privileged bridge controllers can request zero-WASM Layer 4 relays for active media sessions
The platform SHALL allow a privileged bridge controller to allocate dynamic Layer 4 bridge endpoints that the host relays directly without invoking WASM for every packet.

#### Scenario: A media bridge is created for an active session
- **WHEN** a privileged bridge controller requests a new bridge between live endpoints
- **THEN** the host allocates the required ports and relay tasks
- **AND** forwards traffic directly between those endpoints without per-packet guest execution
