## ADDED Requirements

### Requirement: Event connectors subscribe to external systems and trigger ephemeral mesh targets
The platform SHALL support system connectors that subscribe to external queues or brokers and invoke configured internal targets so user functions remain scale-to-zero.

#### Scenario: A connector receives a message from an external broker
- **WHEN** an event connector reads a message from its upstream source
- **THEN** it invokes the configured internal target through the mesh
- **AND** derives acknowledgement behavior from the target response
