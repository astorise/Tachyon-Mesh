# Proposal: Change 031 - Native HTTP/3 & QUIC Transport

## Context
As HTTP/3 (powered by QUIC over UDP) becomes the new standard for web traffic, modern Service Meshes and API Gateways must support it to eliminate Head-of-Line blocking and improve connection resilience (especially for mobile/edge clients). Anticipating the move of QUIC into the OS kernel, Tachyon Mesh must architect a native UDP/QUIC listener. To strictly maintain our "Zero-Overhead" guarantee, this transport layer must be completely feature-gated at compile time.

## Objective
Integrate the `quinn` (QUIC) and `h3` (HTTP/3) Rust ecosystems into the `core-host`. 
1. Create a Cargo feature `http3` that, when enabled, spawns a UDP socket listener alongside (or instead of) the TCP listener.
2. Terminate the QUIC connection and TLS 1.3 securely.
3. Bridge the parsed HTTP/3 requests into the existing Axum `Router` using an adapter (e.g., passing the `http::Request` to the tower service).

## Scope
- Update `core-host/Cargo.toml` with `http3` feature and optional dependencies (`quinn`, `h3`, `h3-quinn`).
- Modify the host's entry point to concurrently bind a `UdpSocket` if the feature is enabled.
- Implement the QUIC accept loop: accepting incoming UDP streams, performing the TLS 1.3 handshake, and demultiplexing the HTTP/3 streams.
- Feed the resulting requests into the same Axum application state and routing table used by HTTP/1 and HTTP/2.

## Success Metrics
- Compiling without `--features http3` results in a binary completely devoid of UDP/QUIC logic, preserving minimal footprint.
- Compiling with the feature allows a client (like `curl --http3`) to connect to the host over UDP, perform a TLS handshake, and receive a response from a WASM FaaS, with the Axum router remaining completely agnostic to the underlying transport protocol.