# Proposal: Change 029 - Pluggable mTLS Gateway (System FaaS)

## Context
Instead of hardcoding a TCP listener and TLS termination directly into the `core-host` Rust code, we will adhere strictly to our "Everything is a FaaS" philosophy. The host should act merely as a capability provider (cryptography engine). We will create a `system-faas-gateway` that uses standard WASI sockets to listen for incoming network traffic, and a custom WIT interface (`tachyon:crypto/tls`) to delegate the heavy mTLS handshake back to the Host's optimized `rustls` implementation.

## Objective
1. Define a `tachyon:crypto/tls` WIT interface that takes a raw TCP socket and returns a decrypted byte stream.
2. Implement the `rustls` cryptographic backend in the `core-host`, exposed exclusively through this WIT interface (behind a `mtls` Cargo feature flag).
3. Create a System FaaS (`system-faas-gateway.wasm`) that listens on a port (e.g., 8443), accepts incoming TCP connections, calls the Host's TLS capability to authenticate and decrypt them, and then forwards the clean HTTP request to the internal FaaS mesh.

## Scope
- Expand `wit/tachyon.wit` to include the cryptography capability.
- Update `core-host` to implement this WIT trait using `tokio-rustls`.
- Build the `system-faas-gateway` in Rust using `wasi::sockets::tcp`.
- Update the `integrity.lock` so users can choose to deploy this gateway FaaS if they want mTLS, or omit it if they want plain HTTP.

## Success Metrics
- If the FaaS gateway is omitted from the manifest, the Host consumes zero cryptographic overhead.
- If deployed, the Gateway successfully binds to a port, delegates the mTLS handshake to the Host, and securely routes the decrypted traffic to the target FaaS.