## MODIFIED Requirements

### Requirement: Host exposes an HTTP gateway for FaaS functions
The `core-host` runtime SHALL run an `axum` server on `0.0.0.0:8080` and route incoming `GET` and `POST` requests through a catch-all gateway capable of resolving a function name from the URL path.

#### Scenario: Client request targets a deployed guest function
- **WHEN** a client sends a `GET` or `POST` request to `/api/guest-example` or `/api/guest-call-legacy`
- **THEN** the host accepts the request on port `8080`
- **AND** the gateway resolves the final path segment as the requested function name
- **AND** the request is dispatched to the WASM execution path for that function

## ADDED Requirements

### Requirement: Host can fulfill mesh fetch commands emitted by a guest
If the captured guest stdout contains a single line beginning with `MESH_FETCH:`, the host SHALL interpret the remainder as an outbound HTTP URL, perform the fetch on the guest's behalf, and return the fetched response body to the original client.

#### Scenario: Guest asks the host to reach a legacy service
- **WHEN** the guest stdout is `MESH_FETCH:http://legacy-service:8081/ping`
- **THEN** the host issues an outbound HTTP `GET` request to that URL
- **AND** the host returns the fetched response body as the HTTP response
- **AND** a failed outbound request results in a gateway-style error response
