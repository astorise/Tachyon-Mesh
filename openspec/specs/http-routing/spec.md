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

### Requirement: Host can enforce optional per-IP rate limiting at compile time
The `core-host` HTTP gateway SHALL expose a `rate-limit` Cargo feature that compiles in a shared per-IP rate limiting middleware while keeping the default build free of rate-limiting state when the feature is disabled.

#### Scenario: Feature is disabled
- **WHEN** `core-host` is built without `--features rate-limit`
- **THEN** the HTTP router is created without a rate limiting layer
- **AND** the default build carries no runtime rate limiting state

#### Scenario: Feature is enabled
- **WHEN** `core-host` is built with `--features rate-limit`
- **THEN** the HTTP router initializes a shared per-IP limiter with a quota of `100` requests per second
- **AND** requests are evaluated by that limiter before guest execution starts

### Requirement: Host rejects burst traffic with HTTP 429
When the `rate-limit` feature is enabled, the HTTP gateway SHALL resolve the client identity from `X-Forwarded-For` or the peer socket address and reject requests that exceed the configured quota with HTTP `429 Too Many Requests`.

#### Scenario: Same client exceeds the quota
- **WHEN** a single client IP sends `101` requests within one second
- **THEN** the first `100` requests are allowed to continue normally
- **AND** the `101st` request is rejected with HTTP `429 Too Many Requests`
- **AND** the rejection happens before the guest module runs

### Requirement: Host enforces per-route concurrency budgets
The `core-host` HTTP gateway SHALL enforce the sealed `max_concurrency` value for each route with
an asynchronous semaphore so guest execution is queued safely instead of oversubscribing the host.

#### Scenario: Request waits for route capacity
- **WHEN** a request arrives for a sealed route whose current in-flight executions are below
  `max_concurrency`
- **THEN** the host acquires a permit for that route before guest execution starts
- **AND** the permit remains held until the request handler completes
- **AND** the request continues through the normal guest execution path

#### Scenario: Request times out waiting for route capacity
- **WHEN** a request waits longer than five seconds for a permit on the target route
- **THEN** the host does not execute the guest
- **AND** the HTTP response status is `503 Service Unavailable`
- **AND** the response explains that the route is currently saturated

### Requirement: Host exposes route queue depth to privileged autoscaling guests
The `core-host` runtime SHALL track queued waiters for each sealed route
concurrency limiter and expose the current pending queue size through
`tachyon:mesh/scaling-metrics`.

#### Scenario: Waiting requests increase the reported queue depth
- **WHEN** requests are waiting for capacity on `/api/guest-call-legacy`
- **THEN** `core-host` reports the current waiter count as that route's pending
  queue size
- **AND** a privileged System FaaS guest can read that value without inspecting the
  semaphore directly

### Requirement: Host can drive background system autoscaling guests
The `core-host` runtime SHALL start a five-second background tick loop only for
sealed `system` components that implement the `background-system-faas` world, and
it SHALL preserve the component instance across ticks so guest cooldown state stays
in memory.

#### Scenario: No autoscaling guest configured
- **WHEN** the sealed configuration contains no `system` route backed by a
  `background-system-faas` component
- **THEN** `core-host` does not start any autoscaling tick worker

#### Scenario: Background autoscaler patches a mock Kubernetes deployment
- **WHEN** the pending queue size for `/api/guest-call-legacy` rises above the
  autoscaler threshold
- **AND** the sealed configuration includes the system route `/system/k8s-scaler`
- **THEN** `core-host` invokes the guest `on-tick` export every five seconds
- **AND** the guest issues a mock outbound HTTP `PATCH` request for the legacy
  deployment
- **AND** subsequent ticks within the cooldown window do not issue another patch

