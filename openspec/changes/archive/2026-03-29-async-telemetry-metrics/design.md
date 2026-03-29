# Design: Async Request Telemetry

## Summary
`async-telemetry-metrics` adds a fire-and-forget telemetry channel to `core-host` so HTTP handlers never block on metrics aggregation, JSON formatting, or log emission. The request path only publishes lifecycle events, while a dedicated Tokio worker correlates those events and emits one structured JSON record per completed request.

## Telemetry Flow
- `run()` initializes a buffered `tokio::sync::mpsc::channel(10_000)` and stores the sender in `AppState`.
- `faas_handler` creates a `trace_id`, sends `RequestStart`, and always sends `RequestEnd` with the final HTTP status code before returning the response.
- The guest execution path receives the same `trace_id` and cloned sender, then emits `WasmStart` and `WasmEnd` around the actual Wasmtime entrypoint invocation.

## Aggregation
- The background worker maintains a `HashMap<String, RequestState>` keyed by `trace_id`.
- Each entry captures the normalized request path plus request and WASM timestamps as they arrive.
- When `RequestEnd` is received, the worker calculates `total_duration_us`, `wasm_duration_us`, and `host_overhead_us`, prints one JSON line, and removes the entry to avoid leaks.

## Backpressure and Failure Semantics
- HTTP and guest execution code paths use `try_send`, so telemetry drops under pressure instead of delaying the request.
- Requests that terminate before guest execution still emit a metrics record with `wasm_duration_us = 0`.
- Saturating duration math ensures incomplete or partially ordered event streams cannot panic the worker.
