# tracing-metering Specification

## Purpose
TBD - created by archiving change tracing-metering. Update Purpose after archive.
## Requirements
### Requirement: Hosts can configure probabilistic telemetry sampling
The host manifest SHALL allow operators to configure a global telemetry sampling rate that determines whether a request incurs tracing and fuel-metering overhead.

#### Scenario: A request is sampled for telemetry
- **WHEN** an incoming request is selected by the configured sampling rate
- **THEN** the host enables request-specific metering and trace collection for that execution

#### Scenario: A request is not sampled for telemetry
- **WHEN** an incoming request is not selected by the configured sampling rate
- **THEN** the host executes the request without enabling trace generation or instruction counting overhead

### Requirement: Sampled telemetry is exported through a bounded asynchronous queue
The host SHALL enqueue completed sampled telemetry records into a bounded asynchronous channel without blocking request execution, and MAY drop new records when the queue is full.

#### Scenario: The telemetry queue accepts a sampled record
- **WHEN** a sampled request completes
- **AND** the telemetry queue has available capacity
- **THEN** the host formats the trace and metrics payload
- **AND** pushes it onto the queue without blocking the request path

#### Scenario: The telemetry queue is saturated
- **WHEN** a sampled request completes
- **AND** the telemetry queue is full
- **THEN** the host drops the telemetry payload instead of blocking or exhausting memory

### Requirement: Metering data is flushed by a background system FaaS
The host SHALL run a background exporter that consumes telemetry records from the queue and forwards them to a system FaaS without delaying primary request handling.

#### Scenario: A telemetry batch is exported
- **WHEN** the background exporter drains one or more telemetry records from the queue
- **THEN** it invokes the metering system FaaS with the batch payload
- **AND** the export path runs independently from primary request execution threads

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

### Requirement: Wasm fuel consumption is reported out-of-band after execution
The `core-host` SHALL configure `wasmtime::Config` with fuel consumption enabled, read the total fuel from the `Store` once a FaaS module completes, and emit a fire-and-forget `tachyon.telemetry.usage` event containing `tenant_id`, `module_id`, and `fuel_consumed` to the internal event bus, without adding any synchronous step to the request path.

#### Scenario: Metering event is emitted without blocking the request
- **WHEN** a Wasm module finishes processing a request
- **THEN** the host reads the fuel counter from the module's `Store`
- **AND** emits a `tachyon.telemetry.usage` event with `tenant_id`, `module_id`, and `fuel_consumed`
- **AND** the request response is returned to the client with no metering-induced latency overhead

### Requirement: system-faas-metering aggregates usage events in the background
`system-faas-metering` SHALL operate strictly as an out-of-band consumer that batches `tachyon.telemetry.usage` events in memory and flushes the aggregated billing data to persistent storage (or Prometheus) on a periodic interval (default 60 seconds).

#### Scenario: Background metering survives a downstream outage
- **WHEN** `system-faas-metering` is consuming usage events
- **AND** the persistent storage backend is temporarily unavailable
- **THEN** the metering FaaS continues to accumulate batches in memory up to its bounded buffer
- **AND** the FaaS retries the flush on the next interval rather than back-pressuring the request path
- **AND** the host's request latency remains unaffected

