# faas-observability Specification

## Purpose
Define the guest-side structured logging contract and the host-side forwarding behavior for FaaS observability without requiring guest network exporters.
## Requirements
### Requirement: FaaS SDK macro initializes lightweight JSON logging
The workspace SHALL provide a `faas-sdk` proc-macro crate exposing `#[faas_handler]`, and that macro SHALL inject a `tracing_subscriber` configured to emit JSON logs to `stdout` at the start of the annotated guest handler.

#### Scenario: Annotated guest handler enables structured logging
- **WHEN** a guest entrypoint is annotated with `#[faas_sdk::faas_handler]`
- **THEN** the generated handler initializes JSON-formatted tracing output to standard output before the handler body runs
- **AND** the resulting function remains invocable as the guest entrypoint under WASI

### Requirement: Guest functions can emit structured telemetry without direct network exporters
Guest functions SHALL be able to depend on `tracing`, use the `faas-sdk` macro on their entrypoint, and emit at least one structured log event that is written to `stdout` alongside the function execution.

#### Scenario: Guest logic emits a structured info event
- **WHEN** the instrumented guest function processes a request
- **THEN** the guest writes at least one JSON log line representing a `tracing::info!` event to `stdout`
- **AND** the guest still writes its final response payload to `stdout`

### Requirement: Host separates guest telemetry from guest response output
After guest execution, `core-host` SHALL read the captured `MemoryWritePipe`, parse line-delimited JSON log entries, forward recognized log entries into the host tracing pipeline, and return only non-log output as the HTTP response body.

#### Scenario: Host forwards guest logs and preserves the response body
- **WHEN** a guest writes JSON tracing events followed by a plain-text response to `stdout`
- **THEN** the host parses and forwards the JSON log lines through its own logging
- **AND** the host excludes those log lines from the returned HTTP body
- **AND** the host returns the remaining plain-text output as the function response

### Requirement: Host emits non-blocking request timing telemetry
The `core-host` runtime SHALL publish request lifecycle timing events through a buffered `tokio::sync::mpsc` channel using `try_send`, so HTTP handlers never await telemetry work on the request path.

#### Scenario: Request timing events stay off the critical path
- **WHEN** `core-host` processes a sealed guest route
- **THEN** it assigns a unique `trace_id` to the request
- **AND** it sends `RequestStart` and `RequestEnd` events through a buffered telemetry channel using `try_send`
- **AND** the guest execution path sends `WasmStart` and `WasmEnd` events through the same channel without awaiting the worker

### Requirement: Host reports correlated request duration metrics
A background telemetry worker SHALL aggregate request lifecycle events by `trace_id` and emit one JSON metrics record per completed request.

#### Scenario: Completed guest request produces total, wasm, and host overhead metrics
- **WHEN** the telemetry worker receives `RequestStart`, `WasmStart`, `WasmEnd`, and `RequestEnd` for the same `trace_id`
- **THEN** it computes `total_duration_us` from request start to request end
- **AND** it computes `wasm_duration_us` from wasm start to wasm end
- **AND** it computes `host_overhead_us` as `total_duration_us - wasm_duration_us`
- **AND** it prints a JSON record containing `trace_id`, `total_duration_us`, `wasm_duration_us`, and `host_overhead_us`
- **AND** it removes the aggregated request state for that `trace_id`

#### Scenario: Requests without guest execution still produce a metrics record
- **WHEN** a request completes before any `WasmStart` or `WasmEnd` event is recorded
- **THEN** the telemetry worker still emits a JSON metrics record for the `trace_id`
- **AND** the record reports `wasm_duration_us` as `0`
- **AND** the record reports `host_overhead_us` equal to `total_duration_us`

### Requirement: System FaaS can read host telemetry snapshots through a privileged world
The workspace SHALL provide a `system-faas-guest` component world that imports
`tachyon:telemetry/reader` and `tachyon:mesh/scaling-metrics`, and `core-host`
SHALL only satisfy those imports for routes sealed as role `system`.

#### Scenario: System autoscaling metrics route exposes pending queue depth
- **WHEN** a request targets the sealed system route `/metrics/scaling`
- **AND** the corresponding guest component imports `tachyon:mesh/scaling-metrics`
- **THEN** `core-host` instantiates the component with the privileged linker
- **AND** the guest can read the pending queue size for `/api/guest-call-legacy`
- **AND** the guest returns Prometheus text containing that queue depth

### Requirement: Host sheds privileged telemetry routes under heavy business load
`core-host` SHALL track active requests and reject sealed `system` routes once active load passes the configured threshold, so normal guest traffic keeps priority over telemetry exports.

#### Scenario: System metrics route is shed under pressure
- **WHEN** the active request count is above the system route load-shedding threshold
- **AND** an incoming request targets a sealed route whose role is `system`
- **THEN** `core-host` returns `503 Service Unavailable`
- **AND** it skips guest execution for that system route

