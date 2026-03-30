## ADDED Requirements

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
