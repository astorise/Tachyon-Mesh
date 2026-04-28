# Proposal: Low-Overhead Distributed Tracing (W3C Trace Context)

## Context
Tachyon Mesh processes requests asynchronously across multiple boundaries: HTTP/3 Ingress -> Core Host -> Queue/Buffer FaaS -> Worker FaaS (e.g., AI Inference) -> Logger FaaS. Currently, when an error occurs or latency spikes, logs are isolated per Wasm module. It is impossible to correlate logs from `system-faas-buffer` with the downstream logs in `system-faas-ai` for a single user request.

## Proposed Solution
We will implement the **W3C Trace Context standard** with a strict focus on zero-blocking overhead:
1. **Ingress Generation:** When `core-host` accepts an HTTP request (in `server_h3.rs`), it looks for an incoming `traceparent` header. If absent, it generates a fast, random 16-byte Trace ID and 8-byte Span ID.
2. **WASI Propagation:** When instantiating a Wasm module (in `Store` or `InstancePre`), the host injects the `TRACEPARENT` into the module's WASI environment variables.
3. **SDK Auto-Instrumentation:** The Rust `faas-sdk` will be updated so that the `logger` and `metrics` macros automatically read the `TRACEPARENT` environment variable and append it to the IPC telemetry payload.
4. **Log Indexing:** The `system-faas-logger` will output this ID, allowing tools like Grafana/Loki to filter all logs globally by `trace_id`.

## Objectives
- Achieve end-to-end request visibility across the distributed Edge mesh.
- Maintain strict performance: generating and passing an environment variable string adds negligible (< 1 microsecond) latency to cold/warm starts.
- Adhere to the industry-standard OpenTelemetry format for future compatibility.