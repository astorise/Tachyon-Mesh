# http-routing Specification

## Purpose
TBD - created by archiving change http-routing. Update Purpose after archive.
## Requirements
### Requirement: Host exposes an HTTP gateway for FaaS functions
The `core-host` runtime SHALL run an `axum` server on `0.0.0.0:8080` and route incoming `GET` and `POST` requests through a catch-all gateway capable of resolving a function name from the URL path.

#### Scenario: Client request targets a deployed guest function
- **WHEN** a client sends a `GET` or `POST` request to `/api/guest-example` or `/api/guest-call-legacy`
- **THEN** the host accepts the request on port `8080`
- **AND** the gateway resolves the final path segment as the requested function name
- **AND** the request is dispatched to the WASM execution path for that function

### Requirement: Host passes request payloads through request-scoped WASI pipes
For each incoming request, `core-host` SHALL create a fresh WASI context that attaches the HTTP request body to a `MemoryReadPipe` and captures guest standard output with a `MemoryWritePipe`.

#### Scenario: Request body becomes guest standard input
- **WHEN** the host prepares execution for a single HTTP request
- **THEN** the request body bytes are written into a virtual WASI stdin pipe for that request
- **AND** a fresh virtual WASI stdout pipe is attached to capture the guest output
- **AND** the WASI context is isolated from other requests

### Requirement: Guest response is returned from captured standard output
The guest module SHALL read its input from standard input, write its response to standard output, and the host SHALL return the captured stdout bytes as the HTTP response body after guest execution completes.

#### Scenario: Guest stdout becomes the HTTP response
- **WHEN** the guest reads the request payload from stdin and writes a response to stdout
- **THEN** the host invokes the guest entrypoint in the request-scoped WASI context
- **AND** the host reads the captured stdout bytes after execution
- **AND** the host returns those bytes as the HTTP response body

### Requirement: Host can fulfill mesh fetch commands emitted by a guest
If the captured guest stdout contains a single line beginning with `MESH_FETCH:`, the host SHALL interpret the remainder as an outbound HTTP target, perform the fetch on the guest's behalf, and return the fetched response body to the original client.

#### Scenario: Guest asks the host to reach a legacy service
- **WHEN** the guest stdout is `MESH_FETCH:http://legacy-service:8081/ping`
- **THEN** the host issues an outbound HTTP `GET` request to that URL
- **AND** the host returns the fetched response body as the HTTP response
- **AND** a failed outbound request results in a gateway-style error response

#### Scenario: Guest asks the host to recurse through another sealed mesh route
- **WHEN** the guest stdout is `MESH_FETCH:/api/guest-loop`
- **THEN** the host resolves the relative route against its own HTTP listener
- **AND** the host injects the decremented `X-Tachyon-Hop-Limit` header into the outbound request
- **AND** the host returns the downstream response status and body to the original client

### Requirement: Host enforces a request hop limit for inbound and outbound mesh traffic
The `core-host` gateway SHALL track a request-scoped hop limit using the `X-Tachyon-Hop-Limit` header so distributed routing loops are rejected before they can exhaust host resources.

#### Scenario: Client omits the hop-limit header
- **WHEN** a client sends a request without `X-Tachyon-Hop-Limit`
- **THEN** the host assigns the request a default hop limit of `10`
- **AND** the request continues through normal route resolution

#### Scenario: A loop exhausts the remaining hops
- **WHEN** an inbound request arrives with `X-Tachyon-Hop-Limit: 0`
- **THEN** the host rejects the request before guest execution starts
- **AND** the HTTP response status is `508 Loop Detected`
- **AND** the response body explains that the routing loop exceeded the hop limit

