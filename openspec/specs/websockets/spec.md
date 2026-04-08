# WebSockets

## Purpose
Define how Tachyon exposes WebSocket resources to guests, lets routes opt into upgrades, and binds upgraded sockets into Wasmtime without blocking the host runtime.

## Requirements
### Requirement: The WIT contract exposes a bidirectional WebSocket connection resource
The Tachyon WIT package SHALL define a WebSocket connection resource with typed frame variants and an `on-connect` guest entrypoint for bidirectional messaging.

#### Scenario: A guest exports the WebSocket handler
- **WHEN** a WebSocket-capable guest is built against the Tachyon WIT package
- **THEN** it can export `on-connect` and use a connection resource to send and receive typed frames

### Requirement: Routes explicitly opt into WebSocket upgrades
The integrity manifest SHALL allow a route or target to declare that it expects a WebSocket upgrade instead of standard HTTP-only request handling.

#### Scenario: A route is configured for WebSocket traffic
- **WHEN** an operator marks a target as `websocket: true`
- **THEN** the host knows that requests to that target require a WebSocket upgrade path

### Requirement: The host upgrades opted-in requests and binds the socket into Wasmtime
When the WebSocket feature is enabled, the host SHALL upgrade opted-in requests, expose the live socket through the Wasmtime resource table, and drive the guest `on-connect` handler asynchronously.

#### Scenario: A WebSocket route receives an upgrade request
- **WHEN** a request targets a WebSocket-enabled route
- **AND** the incoming request includes a valid WebSocket upgrade
- **THEN** the host upgrades the connection
- **AND** passes a socket-backed resource into the guest `on-connect` function
- **AND** suspends guest receive operations asynchronously without blocking an OS thread
