# Specifications: Asynchronous Telemetry Architecture

## 1. The Telemetry Channel
- **Type:** `tokio::sync::mpsc::channel` with a capacity of `10_000` (to handle high-throughput bursts without blocking the sender).
- **Data Structure:** The channel will transmit a `TelemetryEvent` enum.

    pub enum TelemetryEvent {
        RequestStart { trace_id: String, path: String, timestamp: std::time::Instant },
        WasmStart { trace_id: String, timestamp: std::time::Instant },
        WasmEnd { trace_id: String, timestamp: std::time::Instant },
        RequestEnd { trace_id: String, status: u16, timestamp: std::time::Instant },
    }

## 2. Trace Correlation
- Every incoming HTTP request must be assigned a unique `trace_id` (e.g., using the `uuid` crate, v4).
- This `trace_id` is passed along the Axum request state and included in every `TelemetryEvent` sent to the channel.

## 3. The Background Worker
- A `tokio::spawn` task MUST run an infinite loop calling `receiver.recv().await`.
- It maintains an internal `HashMap<String, RequestMetrics>` keyed by `trace_id` to aggregate events.
- When it receives a `RequestEnd` event, it:
  1. Calculates `total_duration = RequestEnd.timestamp - RequestStart.timestamp`.
  2. Calculates `wasm_duration = WasmEnd.timestamp - WasmStart.timestamp`.
  3. Calculates `host_overhead = total_duration - wasm_duration`.
  4. Removes the entry from the HashMap to prevent memory leaks.
  5. Formats and prints a JSON string containing these calculated metrics to `stdout`.

## 4. Instrumentation Points (Axum Handler)
- **Point A (Start):** Immediately upon entering the Axum handler.
- **Point B (Wasm Start):** Right before calling `handle_request` on the Component Model.
- **Point C (Wasm End):** Right after `handle_request` returns.
- **Point D (End):** Right before the Axum handler returns the HTTP Response.