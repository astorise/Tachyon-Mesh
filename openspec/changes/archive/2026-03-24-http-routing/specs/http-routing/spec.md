## ADDED Requirements

### Requirement: Host exposes an HTTP gateway for FaaS functions
The `core-host` runtime SHALL run an `axum` server on `0.0.0.0:8080` and route incoming requests through a catch-all gateway capable of resolving a function name from the URL path.

#### Scenario: Client request targets a deployed guest function
- **WHEN** a client sends an HTTP request to `/api/guest-example`
- **THEN** the host accepts the request on port `8080`
- **AND** the gateway resolves `guest-example` as the requested function name
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
