# Implementation Tasks

## Phase 1: Core Host Ingress
- [x] `uuid` and `rand` are already in `core-host/Cargo.toml`; no new deps needed.
- [x] Add `trace_context_for_request(headers)` in `core-host/src/main.rs` that
      honors a well-formed inbound `traceparent` (W3C version 00, 32-hex trace
      id, 16-hex span id, 2-hex flags; rejects all-zero ids) and otherwise
      mints a fresh one via the existing `generate_traceparent`.

## Phase 2: WASI Boundary Propagation
- [x] Add `add_route_environment_with_trace` (with the existing
      `add_route_environment` as a no-trace shim) that sets `TRACEPARENT` on
      the guest's WASI environment. Wired at the two real-execution sites:
      `execute_legacy_guest_with_sync_file_capture` and
      `execute_legacy_guest_with_stdio`.
- [x] Prewarm path stays traceparent-less since there is no real request.

## Phase 3: FaaS SDK Auto-Instrumentation
- [ ] Reading `TRACEPARENT` from the SDK side and stamping it into emitted
      log/metric payloads is left as a small follow-up; the host now exposes
      the env var and any guest can read it directly via `std::env::var`. The
      SDK macro change will land alongside the inference-queue work in
      Session D so the trace id surfaces consistently across the inference
      hop.

## Phase 4: Validation
- [x] Three unit tests pin the predicate behavior:
  `trace_context_honors_well_formed_inbound_traceparent`,
  `trace_context_rejects_malformed_inbound_and_mints_fresh`,
  `trace_context_mints_fresh_when_header_absent`.
- [ ] Manual end-to-end (send a request with/without `traceparent` to a real
      host and verify it shows up in the guest log) is left for the homelab
      smoke test.
