# Tasks: Change 014 Implementation

## 1. Telemetry Runtime

- [x] 1.1 Add `core-host/src/telemetry.rs` with the `TelemetryEvent` enum, buffered channel initialization, and a background worker entrypoint.
- [x] 1.2 Add the `uuid` dependency required to generate unique `trace_id` values for every request.
- [x] 1.3 Aggregate lifecycle events in a `HashMap<String, RequestState>` and emit a JSON metrics record containing `trace_id`, `total_duration_us`, `wasm_duration_us`, and `host_overhead_us`.

## 2. Host Instrumentation

- [x] 2.1 Inject the telemetry sender into `AppState` so Axum handlers and guest execution code can share the same channel.
- [x] 2.2 Instrument `faas_handler` to emit `RequestStart` and `RequestEnd` events with a per-request `trace_id`.
- [x] 2.3 Instrument guest execution to emit `WasmStart` and `WasmEnd` with `try_send`, keeping telemetry fully non-blocking.

## 3. Verification

- [x] 3.1 Add or update tests covering telemetry aggregation and request instrumentation behavior.
- [x] 3.2 Verify the change with local Rust checks and `openspec validate --all`.
