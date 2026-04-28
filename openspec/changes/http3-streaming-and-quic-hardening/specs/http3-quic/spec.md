## ADDED Requirements

### Requirement: HTTP/3 request bodies are streamed without full buffering
The `core-host` HTTP/3 server (`server_h3.rs`) SHALL bridge incoming `h3` data streams directly into an asynchronous `axum::body::Body` so that request payloads are forwarded chunk-by-chunk to downstream Wasm modules or storage brokers without being accumulated in host RAM.

#### Scenario: Large GGUF upload streams without OOM
- **WHEN** a client uploads a multi-gigabyte GGUF model body over HTTP/3
- **THEN** the host forwards body chunks to the configured downstream sink as they arrive
- **AND** the host's RSS does not grow with the cumulative size of the request body
- **AND** the upload completes successfully without an OOM crash

### Requirement: QUIC server is hardened against connection-exhaustion attacks
The `quinn` server configuration SHALL set explicit idle timeouts and concurrent-connection / concurrent-stream limits so that hung or malicious QUIC peers cannot exhaust host resources.

#### Scenario: Slowloris-style QUIC peer is reaped
- **WHEN** a client opens a QUIC connection but stops sending data for longer than the configured idle timeout
- **THEN** the host closes the connection and releases its associated state
- **AND** new legitimate connections are still accepted
- **AND** the configured concurrency caps prevent any single peer from monopolising server resources
