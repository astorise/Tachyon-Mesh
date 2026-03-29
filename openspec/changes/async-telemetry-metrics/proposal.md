# Proposal: Change 014 - Asynchronous Telemetry & Performance Metrics

## Context
To market Tachyon Mesh effectively and prove its "Zero-Overhead" claims against traditional Service Meshes (Istio/Envoy), we must rigorously measure our execution times. However, adding observability (logging, metric calculations, JSON formatting) synchronously within the critical HTTP request path introduces the "Observer Effect," artificially inflating the FaaS latency.

## Objective
Implement an asynchronous, non-blocking telemetry pipeline in the `core-host`. We will use a `tokio::sync::mpsc` channel to pass lightweight timing events from the HTTP workers to a dedicated background telemetry worker. This worker will calculate the exact Host overhead versus pure WASM execution time and output structured JSON metrics for benchmarking and monitoring.

## Scope
- Define `TelemetryEvent` structures (e.g., `RequestStarted`, `WasmExecutionStarted`, `WasmExecutionFinished`, `RequestCompleted`).
- Create a globally accessible or state-injected `mpsc::Sender`.
- Spawn a `tokio` background task at startup to consume the `mpsc::Receiver`.
- Instrument the Axum router and Wasmtime execution block with `std::time::Instant` to measure precise durations.
- Calculate and log the "Tachyon Overhead" (Total Request Time minus WASM Execution Time).

## Success Metrics
- HTTP response times do not degrade after adding telemetry.
- Every HTTP request generates a structured JSON log line in the background containing: `trace_id`, `total_duration_us`, `wasm_duration_us`, and `host_overhead_us`.
- The system correctly correlates the multiple steps of a single request using a unique `trace_id`.