## ADDED Requirements

### Requirement: Pressure control adapts monitoring and buffers overload through memory and disk tiers
The host SHALL minimize monitoring overhead when no peers are available and SHALL buffer overload through bounded RAM and disk spillover before failing requests when overflow is unavailable.

#### Scenario: Local pressure rises with no remote overflow target
- **WHEN** the node detects high local pressure and no healthy peer can accept overflow traffic
- **THEN** it reduces unnecessary monitoring work on single-node deployments
- **AND** queues requests through RAM first, then disk spillover, before rejecting additional load
