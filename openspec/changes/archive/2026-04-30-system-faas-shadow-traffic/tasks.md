# Implementation Tasks

## Phase 1: Configuration & Routing
- [x] Add the `shadow_target` optional field to the routing schema parser.
- [x] Update the `core-host` dispatcher to conditionally clone the request and emit the `ShadowEvent` via IPC.

## Phase 2: The System FaaS
- [x] Bootstrap `systems/system-faas-shadow-proxy` as a standard Wasm module.
- [x] Implement the logic to execute the shadowed Wasm module via host-provided loopback calls.
- [x] Implement the Diffing Engine (comparing status codes, headers, and payload hashes).

## Phase 3: Telemetry Integration
- [x] Wire `system-faas-shadow-proxy` to send divergence metrics to `system-faas-otel` (if loaded).

## Phase 4: Validation
- [x] **Overhead Test:** Run a benchmark with and without `shadow_target` configured. Verify that the p99 latency of the *client response* remains identical, proving the asynchronous decoupling works.
