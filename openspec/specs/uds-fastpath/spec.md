# uds-fastpath Specification

## Purpose
Define the intra-node Unix domain socket fast path used by Tachyon hosts to avoid loopback TCP overhead while preserving mesh routing semantics, fallback behavior, and local discovery safety.
## Requirements
### Requirement: Each local host publishes a discoverable UDS endpoint
Every Tachyon host running on a node SHALL publish a Unix domain socket and matching metadata file in a shared discovery directory so local peers can discover fast-path endpoints.

#### Scenario: A host starts on a shared node
- **WHEN** a host boots with access to the shared discovery directory
- **THEN** it creates a unique Unix domain socket endpoint
- **AND** writes metadata that includes its network identity and supported protocols

### Requirement: Local peer discovery prefers a matching UDS endpoint
The mesh router SHALL inspect the shared discovery directory for a Unix domain socket whose metadata matches the destination peer before attempting a TCP connection. Discovery results SHALL be cached for a bounded TTL so repeated local fetches do not rescan and parse every peer metadata file on each request.

#### Scenario: A local peer is discoverable through metadata
- **WHEN** a request targets a peer IP with a matching metadata entry in the discovery directory
- **THEN** the router resolves the peer to the associated Unix domain socket path
- **AND** subsequent requests within the cache TTL reuse the cached peer snapshot instead of rescanning the discovery directory

#### Scenario: Cached peer metadata expires
- **WHEN** the peer discovery cache TTL has elapsed
- **THEN** the router refreshes the peer snapshot from the discovery directory before selecting a fast-path endpoint
- **AND** stale metadata whose socket path no longer exists is removed from discovery

### Requirement: UDS clients are reused per peer socket
The mesh transport SHALL cache UDS HTTP clients by socket path so repeated calls to the same local peer reuse keep-alive connections instead of constructing a new client and reconnecting for every request.

#### Scenario: Repeated component outbound HTTP calls target the same local peer
- **WHEN** a component calls `tachyon:mesh/outbound-http.send-request` repeatedly for an internal target resolved to the same UDS socket
- **THEN** the host reuses the cached blocking UDS client for that socket
- **AND** the original HTTP method, headers, and body are forwarded over UDS

### Requirement: Transport falls back to TCP when the fast path is unavailable
The mesh router SHALL use the Unix domain socket for local traffic when the socket is reachable and SHALL fall back to the normal TCP path when no match exists or the socket connection fails.

#### Scenario: The fast-path socket is reachable
- **WHEN** the router resolves a peer to a healthy Unix domain socket
- **THEN** it establishes the peer connection over UDS while preserving the existing mesh protocol stack

#### Scenario: The fast-path socket is missing or stale
- **WHEN** the router cannot resolve or connect to a usable Unix domain socket for the peer
- **THEN** it retries the outbound connection through the standard TCP path without hanging the caller
- **AND** the failed peer and its cached clients are evicted so the next request can rediscover a healthy endpoint

### Requirement: Component outbound HTTP can use the UDS fast path
The `tachyon:mesh/outbound-http` host import SHALL attempt the UDS fast path for internal mesh targets after in-process dispatch is unavailable and before falling back to loopback TCP.

#### Scenario: Component outbound HTTP sends a non-GET internal request
- **WHEN** a component guest calls `outbound-http.send-request("POST", "http://mesh/internal/echo", headers, body)`
- **AND** the internal target cannot be handled by in-process dispatch
- **AND** a matching local peer UDS endpoint is discoverable
- **THEN** the host forwards the POST request, headers, and body over UDS
- **AND** the component receives the UDS response status, headers, and body
