# Implementation Tasks

## Phase 1: Wasmtime Engine Update
- [ ] Open the `core-host` module where `wasmtime::Engine` is configured.
- [ ] Call `consume_fuel(true)` on the `Config`.
- [ ] Update the `Store` instantiation logic to inject fuel before calling the Wasm function.

## Phase 2: Host Telemetry Hook
- [ ] In the FaaS execution pipeline, extract the consumed fuel using `store.fuel_consumed()`.
- [ ] Format an asynchronous event payload containing the module metadata and the fuel metric.
- [ ] Dispatch this payload to the internal async channel/bus without awaiting a response.

## Phase 3: Metering FaaS Refactoring
- [ ] Update `system-faas-metering` to act as an event subscriber rather than a blocking API.
- [ ] Implement the in-memory batching logic (aggregation by tenant).
- [ ] Implement the periodic flush to persistent storage or Prometheus metrics exporter.

## Phase 4: Validation
- [ ] Deploy a CPU-intensive Wasm module.
- [ ] Hit the endpoint and verify the response time is unaffected.
- [ ] Check the logs/database of `system-faas-metering` a minute later to ensure the fuel consumption was accurately recorded in the background.