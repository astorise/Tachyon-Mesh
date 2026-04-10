# Data Events

## Purpose
Define how Tachyon decouples storage uploads and database-style outbox mutations from downstream mesh execution by using system connectors that proxy or poll upstream systems asynchronously.

## Requirements
### Requirement: Data-plane events are dispatched asynchronously through system connectors
The platform SHALL decouple database and storage mutations from mesh invocation by using system connectors that poll or proxy upstream systems and dispatch events asynchronously.

#### Scenario: A system connector observes a new data event
- **WHEN** a connector detects a new outbox record or completes a proxied storage write
- **THEN** it forwards the corresponding event into the mesh asynchronously
- **AND** avoids coupling the upstream transaction or client request to downstream function execution

### Requirement: The CDC poller acknowledges only successful downstream delivery
The platform SHALL keep outbox events pending until the target route accepts the payload successfully.

#### Scenario: The background CDC connector processes an outbox row
- **WHEN** the connector claims an outbox event from the host outbox store
- **THEN** it POSTs the payload to the configured target route
- **AND** it acknowledges the outbox row only after the target responds with `200 OK`

### Requirement: The storage proxy forwards object uploads before emitting mesh metadata
The platform SHALL persist the upstream object write before it emits an internal upload event to the mesh.

#### Scenario: A client uploads an object through the proxy
- **WHEN** the proxy receives a `PUT` request with an object body
- **THEN** it forwards the bytes to the configured upstream bucket endpoint with the required authorization headers
- **AND** it buffers a mesh event containing the uploaded object metadata for asynchronous replay
