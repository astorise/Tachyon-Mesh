# Implementation Tasks

## Phase 1: Body Streaming Logic (`core-host`)
- [ ] Remove the `while let Some(chunk) = stream.recv_data()` loop in `server_h3.rs`.
- [ ] Implement a `Stream`-to-`Body` adapter to pipe `h3` data chunks into Axum.
- [ ] Update `handle_http3_request` to construct the Axum request using this new streaming body.

## Phase 2: QUIC Configuration Hardening (`core-host`)
- [ ] Modify `build_quinn_server_config` to include `TransportConfig`.
- [ ] Set `max_idle_timeout` to `Duration::from_secs(30)`.
- [ ] Set `initial_max_bidi_streams` to `100`.

## Phase 3: Response Streaming
- [ ] Ensure `send_http3_response` also handles streaming responses from Axum instead of collecting them via `.collect().await`.

## Phase 4: Validation
- [ ] **Test OOM:** Simulate a 1GB upload through an HTTP/3 route and verify that `core-host` RAM usage remains stable under 100MB.
- [ ] **Test Timeout:** Verify that an idle QUIC connection is automatically closed by the host after 30 seconds.