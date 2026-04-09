# Specifications: Async Logging Pipeline

## 1. Host-Side Interception
The `core-host` configures the Wasmtime `Linker` to redirect `stdout` and `stderr` to a `tokio::sync::mpsc` channel instead of the standard process output.

## 2. The `system-faas-logger`
This System FaaS is a Singleton that listens to the log channel. 
- It aggregates logs with metadata: `{ timestamp, faas_id, tenant_id, stream_type, message }`.
- It performs batching and compression before writing to the local disk or sending to a remote endpoint (Change 029 mTLS).

## 3. Backpressure Management
If the in-memory log buffer is full (e.g., during a "log storm"), the host will start dropping logs (Last-In-First-Out) to prioritize system stability over log completeness.