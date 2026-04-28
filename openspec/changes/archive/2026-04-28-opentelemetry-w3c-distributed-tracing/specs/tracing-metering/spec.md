## ADDED Requirements

### Requirement: HTTP/3 ingress generates or honors a W3C traceparent
When `core-host` accepts an incoming HTTP/3 request, it SHALL adopt the value of the incoming `traceparent` header if present and well-formed, and SHALL otherwise generate a fresh 16-byte Trace ID and 8-byte Span ID following the W3C Trace Context specification.

#### Scenario: Incoming traceparent is honored
- **WHEN** an HTTP/3 request arrives with a syntactically valid `traceparent` header
- **THEN** the host preserves the incoming Trace ID
- **AND** generates a new child Span ID linked to the incoming trace

#### Scenario: Missing traceparent triggers fresh generation
- **WHEN** an HTTP/3 request arrives without a `traceparent` header
- **THEN** the host generates a 16-byte Trace ID and an 8-byte Span ID
- **AND** records the assigned IDs for downstream propagation

### Requirement: Trace context is propagated into Wasm modules via WASI environment
When the host instantiates a Wasm module to handle a request, it SHALL inject the active trace context as a `TRACEPARENT` environment variable in the module's WASI environment.

#### Scenario: Wasm guest sees TRACEPARENT in its environment
- **WHEN** the host invokes a FaaS module to handle a request with an active trace
- **THEN** the module's WASI environment contains a `TRACEPARENT` variable encoding the W3C `traceparent` value
- **AND** subsequent FaaS hops downstream of this module observe the same Trace ID

### Requirement: faas-sdk auto-instruments logs and metrics with the active trace
The Rust `faas-sdk` SHALL update its logger and metrics macros to read `TRACEPARENT` from the environment and append the trace identifier to every emitted log line and telemetry payload.

#### Scenario: Log line carries the trace identifier
- **WHEN** a FaaS module emits a log via the SDK macro while `TRACEPARENT` is set in its environment
- **THEN** the IPC log payload includes the trace identifier
- **AND** `system-faas-logger` outputs the identifier so log indexers can filter all logs of a request by `trace_id`
