# Proposal: Change 052 - Sampled Tracing & Telemetry Queue

## Context
Full-fidelity observability (tracking CPU cycles, RAM, and OpenTelemetry spans for every execution) introduces a massive overhead that neutralizes the extreme performance of the WebAssembly engine. To maintain nanosecond latency while still gaining statistical visibility into the system, we must use Probabilistic Sampling. Furthermore, telemetry data must not block the host; it must be written to an asynchronous queue and processed by a dedicated System FaaS.

## Objective
1. Introduce a `telemetry_sample_rate` configuration in the `core-host` (e.g., `0.001` for 1/1000 requests).
2. For the sampled requests, enable Wasmtime CPU fuel counting, capture precise memory allocations, and generate W3C Trace context.
3. Push the sampled data into an asynchronous, non-blocking queue (Pile).
4. Deploy a `system-faas-metering` component to consume this queue in the background and export the data.

## Scope
- Update `integrity.lock` to include global sampling configuration.
- Implement random sampling logic in the HTTP dispatcher.
- Create the memory MPSC queue and the System FaaS consumer hook.