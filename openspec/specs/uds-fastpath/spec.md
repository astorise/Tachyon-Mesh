# uds-fastpath Specification

## Purpose
TBD - created by archiving change uds-fastpath. Update Purpose after archive.
## Requirements
### Requirement: Each local host publishes a discoverable UDS endpoint
Every Tachyon host running on a node SHALL publish a Unix domain socket and matching metadata file in a shared discovery directory so local peers can discover fast-path endpoints.

#### Scenario: A host starts on a shared node
- **WHEN** a host boots with access to the shared discovery directory
- **THEN** it creates a unique Unix domain socket endpoint
- **AND** writes metadata that includes its network identity and supported protocols

### Requirement: Local peer discovery prefers a matching UDS endpoint
The mesh router SHALL inspect the shared discovery directory for a Unix domain socket whose metadata matches the destination peer before attempting a TCP connection.

#### Scenario: A local peer is discoverable through metadata
- **WHEN** a request targets a peer IP with a matching metadata entry in the discovery directory
- **THEN** the router resolves the peer to the associated Unix domain socket path

### Requirement: Transport falls back to TCP when the fast path is unavailable
The mesh router SHALL use the Unix domain socket for local traffic when the socket is reachable and SHALL fall back to the normal TCP path when no match exists or the socket connection fails.

#### Scenario: The fast-path socket is reachable
- **WHEN** the router resolves a peer to a healthy Unix domain socket
- **THEN** it establishes the peer connection over UDS while preserving the existing mesh protocol stack

#### Scenario: The fast-path socket is missing or stale
- **WHEN** the router cannot resolve or connect to a usable Unix domain socket for the peer
- **THEN** it retries the outbound connection through the standard TCP path without hanging the caller

