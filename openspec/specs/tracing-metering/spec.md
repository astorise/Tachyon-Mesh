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
The host's background metering exporter SHALL own the in-memory aggregation of usage events: it accumulates `tachyon.telemetry.usage` records drained from the bounded telemetry queue and flushes a batch either when the batch reaches the configured size (`TELEMETRY_EXPORT_BATCH_SIZE`) or when the periodic flush interval elapses (`TELEMETRY_EXPORT_FLUSH_INTERVAL`, default 60 seconds), whichever comes first. The exporter SHALL forward each flushed batch to `system-faas-metering`, which SHALL operate strictly as the out-of-band persistence sink (appending the billing records to durable storage) and SHALL NOT back-pressure the request path.

#### Scenario: Batch flushes on size or interval
- **WHEN** the exporter has accumulated one or more usage records
- **THEN** it flushes once the batch reaches `TELEMETRY_EXPORT_BATCH_SIZE`
- **AND** it also flushes a non-empty batch when the flush interval (default 60 s) elapses
- **AND** flushing forwards the batch to `system-faas-metering` off the request path

#### Scenario: Downstream outage does not back-pressure requests
- **WHEN** a metering forward fails because the sink is temporarily unavailable
- **THEN** the exporter continues draining and flushing subsequent batches on each interval
- **AND** the host's request latency remains unaffected

### Requirement: Metering batches are durably staged before export
Before forwarding a metering batch, the host SHALL persist each record to the durable `metering_outbox` store. On a successful forward it SHALL delete those staged entries; on a failed forward the entries SHALL be retained so that no usage record is lost to a host crash or a downstream outage.

#### Scenario: Records survive a crash between staging and export
- **WHEN** the host persists a metering batch to the outbox and then crashes before the forward completes
- **THEN** the staged records remain in the `metering_outbox` table after restart

#### Scenario: Successful export clears the outbox; failure retains it
- **WHEN** a metering batch is forwarded successfully
- **THEN** the host deletes the corresponding `metering_outbox` entries
- **AND** a failed forward instead retains those entries durably for recovery

### Requirement: Retained metering records are re-exported by a retry sweeper
The host SHALL run a background retry sweeper that drains the `metering_outbox` on a bounded cadence (`METERING_OUTBOX_RETRY_INTERVAL`, default 60 seconds): each sweep peeks up to the configured batch limit (`METERING_OUTBOX_RETRY_BATCH_LIMIT`), re-forwards those records through the same metering export path, and deletes them only after the forward succeeds. Records whose re-export fails SHALL remain in the outbox for a later sweep, so staged usage records are eventually delivered once the sink recovers rather than requiring manual recovery.

#### Scenario: Successful retry clears the staged entries
- **WHEN** the sweeper drains an outbox containing previously staged records and the export succeeds
- **THEN** the records are forwarded through the metering export path
- **AND** the corresponding `metering_outbox` entries are deleted

#### Scenario: Failed retry leaves entries for the next sweep
- **WHEN** a sweep's re-export fails because the sink is still unavailable
- **THEN** the peeked entries remain in the `metering_outbox`
- **AND** the sweeper retries them on a subsequent cadence tick without blocking request handling

