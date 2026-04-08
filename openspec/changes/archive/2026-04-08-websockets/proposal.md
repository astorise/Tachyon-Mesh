# Proposal: Change 032 - Stateful WebSockets Gateway

## Context
Modern applications require real-time, bidirectional communication (WebSockets). Unlike stateless HTTP/1 requests or gRPC streams, WebSockets begin as an HTTP request and "upgrade" to a persistent TCP framing protocol. Standard WASM FaaS instances are ephemeral, which conflicts with this stateful requirement. We need a mechanism for a WASM guest to maintain a long-lived bidirectional channel with the client without blocking the Rust host's async event loop.

## Objective
Implement WebSocket support in the `core-host` using Axum's built-in `ws` extractor, gated behind a `websockets` Cargo feature. Define a new WIT interface (`tachyon:network/websocket`) that provides the guest with a resource handle to send and receive frames (Text, Binary, Ping, Pong). The FaaS will run in an asynchronous loop, suspending itself efficiently when waiting for messages.

## Scope
- Update `core-host/Cargo.toml` to enable Axum's `ws` feature optionally.
- Expand `wit/tachyon.wit` to include the `websocket` interface and a `frame` variant type.
- Modify the Axum router: if a route is flagged as a websocket endpoint in `integrity.lock`, Axum intercepts the HTTP Upgrade request, establishes the WebSocket, and then instantiates the WASM module, passing the connection handle.
- Build a `guest-websocket-echo` FaaS to demonstrate persistent bidirectional communication.

## Success Metrics
- Compiling without `--features websockets` produces a proxy that strictly rejects HTTP Upgrade requests, maintaining zero L4 overhead.
- Compiling with the feature allows a frontend application (e.g., standard JS `new WebSocket()`) to connect to the configured route.
- The WASM FaaS successfully receives text frames and echoes them back, maintaining its internal state (memory) across multiple messages until the connection is closed.