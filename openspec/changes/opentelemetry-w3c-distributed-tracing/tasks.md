# Implementation Tasks

## Phase 1: Core Host Ingress
- [ ] Add the `uuid` and `rand` crates to `core-host/Cargo.toml` if not already present.
- [ ] In `server_h3.rs` (or the generic routing middleware), read the incoming `traceparent` HTTP header.
- [ ] If the header is missing, generate a new W3C-compliant string.

## Phase 2: WASI Boundary Propagation
- [ ] Update the `core-host` Wasm dispatcher logic to inject the `TRACEPARENT` string into the WASI environment variables of the spawned instance.
- [ ] If using the new Instance Pooling, ensure the env vars are correctly set per-request on the `Store` state.

## Phase 3: FaaS SDK Auto-Instrumentation
- [ ] Open `faas-sdk/src/lib.rs`.
- [ ] Update the logging and metering functions to capture the `TRACEPARENT` environment variable.
- [ ] Prepend or append the Trace ID to the emitted log string or metric payload.

## Phase 4: Validation
- [ ] Deploy an echo module using the updated `faas-sdk`.
- [ ] Send an HTTP request and verify that the `system-faas-logger` prints the correct `trace_id` in the console.
- [ ] Send an HTTP request *with* a custom `traceparent` header (e.g., from Postman) and verify that the host successfully propagated the client's provided trace ID all the way to the FaaS logs.