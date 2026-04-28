# Implementation Tasks

## Phase 1: Body Streaming Logic (`core-host`)
- [x] Remove the accumulating `while let Some(chunk) = stream.recv_data()` loop in `server_h3.rs`.
- [x] Bridge `h3` data chunks into Axum via `tokio::sync::mpsc` + `tokio_stream::wrappers::ReceiverStream` + `axum::body::Body::from_stream`.
- [x] Split the bidi stream so the recv pump can run as its own task while the send half is retained for writing the response.

## Phase 2: QUIC Configuration Hardening (`core-host`)
- [x] Modify `build_quinn_server_config` to attach a `quinn::TransportConfig`.
- [x] Set `max_idle_timeout` to `Duration::from_secs(30)`.
- [x] Set `max_concurrent_bidi_streams` to `256` and `max_concurrent_uni_streams` to `100`.
- [x] Set `keep_alive_interval` to `Duration::from_secs(15)` so legitimate long-lived connections survive intermediate NAT timeouts.

## Phase 3: Response handling
- [x] Response bodies still go through `BodyExt::collect` because their size is bounded by the route logic, not by external client uploads. The existing flow is preserved; per-frame response streaming is left as a follow-up if a route requires it.

## Phase 4: Validation
- [x] Unit test `quic_transport_config_caps_are_applied` pins the safety constants so a regression to quinn defaults is caught.
- [ ] (Manual) End-to-end: upload a 1 GiB body to an HTTP/3 route and confirm host RSS does not grow with body size. Tracking the steady-state RSS during a real upload is left for the homelab smoke test.
