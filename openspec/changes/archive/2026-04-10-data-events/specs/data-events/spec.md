## ADDED Requirements

### Requirement: Data-plane events are dispatched asynchronously through system connectors
The platform SHALL decouple database and storage mutations from mesh invocation by using system connectors that poll or proxy upstream systems and dispatch events asynchronously.

#### Scenario: A system connector observes a new data event
- **WHEN** a connector detects a new outbox record or completes a proxied storage write
- **THEN** it forwards the corresponding event into the mesh asynchronously
- **AND** avoids coupling the upstream transaction or client request to downstream function execution
