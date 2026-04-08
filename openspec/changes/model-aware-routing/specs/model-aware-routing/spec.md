## ADDED Requirements

### Requirement: AI routing prefers peers that already have the target model hot
The router SHALL consider hot-model state as a first-class placement signal so latency-sensitive inference is not sent to peers that would incur a cold load.

#### Scenario: A real-time request targets a model that is not hot on a remote peer
- **WHEN** the router evaluates overflow candidates for a latency-sensitive model invocation
- **THEN** it keeps the request local or selects only peers that already have the target model loaded
- **AND** avoids sending that request to peers that would need a cold model load
