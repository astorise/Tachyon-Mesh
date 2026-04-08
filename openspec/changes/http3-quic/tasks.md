# Tasks: Change 031 Implementation

**Agent Instruction:** Read the `proposal.md` and `specs.md`. Implement the QUIC listener strictly behind the `http3` feature flag, bridging it to the Axum router.

## [TASK-1] Add Dependencies and Features
1. Update `core-host/Cargo.toml` with the `http3` feature and the `quinn` / `h3` ecosystem crates as optional dependencies.

## [TASK-2] Implement the Quinn Endpoint
1. Create a new module `server_h3.rs` wrapped in `#[cfg(feature = "http3")]`.
2. Write a function that takes the `rustls::ServerConfig` (reused from your Change 029 logic) and binds a `quinn::Endpoint` to a UDP port (e.g., 8443).
3. Implement the `accept()` loop to handle incoming QUIC connections.

## [TASK-3] Bridge H3 Streams to Axum
1. Inside the QUIC connection loop, wrap the connection in `h3::server::Connection`.
2. For each accepted bi-directional stream (which represents an HTTP/3 request), extract the `http::Request`.
3. Use the `tower::ServiceExt::oneshot` method on your cloned Axum `Router` to process the request through your existing FaaS handler.
4. Send the resulting `http::Response` back through the `h3` stream.

## Validation Step
1. Start the host with `cargo run --release --features "http3 mtls"`. Ensure it binds to UDP 8443.
2. Build a simple `guest-hello.wasm` route.
3. Test with a QUIC-enabled client (e.g., `curl --http3 -k https://localhost:8443/api/hello`).
4. Verify the WASM FaaS executes correctly and the response is routed back over UDP without traversing the TCP stack.