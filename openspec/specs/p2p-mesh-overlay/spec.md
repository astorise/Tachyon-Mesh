# p2p-mesh-overlay Specification

## Purpose
TBD - created by archiving change p2p-mesh-overlay-discovery. Update Purpose after archive.
## Requirements
### Requirement: system-faas-mesh-overlay broadcasts hardware heartbeats
The Mesh SHALL provide `system-faas-mesh-overlay`, an optional System FaaS that periodically broadcasts a hardware heartbeat describing the local node's capabilities (for example `gpu_available`, `active_faas_count`, `supported_models`).

#### Scenario: Node advertises its capabilities to peers
- **WHEN** `system-faas-mesh-overlay` is enabled on a node
- **THEN** it periodically broadcasts a heartbeat containing the node's current hardware capabilities and load
- **AND** peers receive the heartbeat and update their routing tables with the advertised capabilities

### Requirement: Mesh overlay maintains a dynamic peer routing table over a secure tunnel
`system-faas-mesh-overlay` SHALL maintain a real-time table of peer nodes, their advertised capabilities, and their current load, and SHALL establish secure peer-to-peer tunnels using mTLS or the Noise protocol so traffic between peers is authenticated and encrypted.

#### Scenario: Overlay tunnel rejects an unauthenticated peer
- **WHEN** a remote node attempts to join the overlay without presenting a valid mTLS or Noise handshake credential
- **THEN** `system-faas-mesh-overlay` refuses the tunnel
- **AND** the offending peer does not appear in the local routing table

### Requirement: Core host delegates overflow requests to capable peers
When the local `core-host` cannot satisfy a request locally (for example because the local accelerator is saturated), it SHALL ask `system-faas-mesh-overlay` for the best peer that can serve it, forward the raw request payload through the secure tunnel, and stream the remote response back to the client transparently.

#### Scenario: Saturated GPU forwards inference to an idle peer
- **WHEN** an inference request arrives on a node whose GPU is saturated
- **AND** `system-faas-mesh-overlay` reports an idle peer with `gpu_available: true` and a matching `supported_model`
- **THEN** the host forwards the request payload over the secure peer tunnel
- **AND** streams the peer's response back to the original client
- **AND** the client observes the response as if it had been served locally

