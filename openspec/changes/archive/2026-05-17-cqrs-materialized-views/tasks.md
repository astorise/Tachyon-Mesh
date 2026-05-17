# Implementation Tasks

- [x] **Task 1: Build the `system-faas-view-builder` SDK**
  - Create a framework within the Logical Plane allowing developers to easily define "View Manifests" (which tables to listen to, and the Rust/Wasm closure to execute to rebuild the view).

- [x] **Task 2: CDC Event Ordering**
  - Ensure the `tachyon:storage/data-events` WIT contract correctly exposes Vector Clocks or monotonically increasing sequence numbers to guarantee the View Builder can resolve out-of-order event delivery.

- [x] **Task 3: Gateway Fast-Path Integration**
  - Update `system-faas-gateway` to expose a protected HTTP/QUIC endpoint (e.g., `GET /v1/views/{view_name}`) that maps directly to a `reddb_direct::get("V:{view_name}")` call.
  - Ensure the Biscuit Auth interceptor validates read-access capabilities *before* serving the materialized view.

- [x] **Task 4: Background Priority Execution**
  - Update the `core-host` Wasmtime scheduler. View Builder FaaS instances are background workers. They should be scheduled with a lower CPU priority than synchronous user-facing FaaS (like the API Gateway or authentication modules) to prevent background materialization from stalling live network requests.
