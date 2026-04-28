## ADDED Requirements

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
