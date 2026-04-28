# Proposal: HTTP/3 Streaming and QUIC Hardening

## Context
The current HTTP/3 implementation in `server_h3.rs` buffers the entire request body into RAM before dispatching it to the Axum router. While functional for small API calls, this approach causes immediate Out-of-Memory (OOM) crashes when handling large payloads (e.g., GGUF model uploads or heavy data streams). Furthermore, the QUIC server uses default parameters, leaving it vulnerable to connection-exhaustion attacks.

## Proposed Solution
1. **Zero-Copy Streaming:** Refactor the request handler to bridge the `h3` data stream directly into an asynchronous `axum::body::Body`. This allows the host to forward data chunks to Wasm modules or storage brokers as they arrive, without accumulating them in memory.
2. **QUIC Defensive Configuration:** Hardened the `quinn` server configuration with explicit idle timeouts and concurrency limits to mitigate "Slowloris" style attacks over UDP.

## Objectives
- Prevent OOM crashes during large data transfers.
- Reduce request latency by enabling "first-byte-processing".
- Enhance Mesh resilience against malicious or hanging QUIC connections.