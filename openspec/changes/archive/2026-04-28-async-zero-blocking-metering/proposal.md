# Proposal: Asynchronous Zero-Blocking Metering

## Context
To offer a Multi-Tenant Edge PaaS, Tachyon Mesh must accurately measure the compute resources consumed by each user (FinOps). However, introducing synchronous calls to a metering database or a dedicated FaaS during the request lifecycle would severely degrade the sub-millisecond invocation latency. 

## Proposed Solution
We will implement an **Out-of-Band Metering Architecture**:
1. **Native Fuel:** Configure the `wasmtime::Config` in the `core-host` to consume "Fuel". This allows the engine to accurately count the exact number of Wasm instructions executed by the CPU with negligible overhead.
2. **Post-Execution Emission:** When a FaaS module finishes executing and drops, the `core-host` reads the total fuel consumed from the `Store`.
3. **Fire-and-Forget Event:** The host emits a telemetry event (`tachyon.telemetry.usage`) containing the `tenant_id`, `module_id`, and `fuel_consumed` to the internal event bus.
4. **Background Aggregation:** `system-faas-metering` operates strictly as a background consumer. It listens to these events, batches them in memory, and flushes the aggregated billing data to persistent storage (or Prometheus) every 60 seconds.

## Objectives
- Achieve deterministic, instruction-level billing precision.
- Guarantee 0ms synchronous latency overhead on the FaaS critical path.
- Decouple the billing logic entirely from the execution logic.