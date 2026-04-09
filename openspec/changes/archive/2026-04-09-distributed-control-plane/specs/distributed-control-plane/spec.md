## ADDED Requirements

### Requirement: Cluster steering logic runs in system control-plane functions rather than the host data plane
The host SHALL expose telemetry and route-update capabilities while system control-plane functions perform gossip, overflow decisions, and buffering policy.

#### Scenario: Local pressure exceeds the healthy threshold
- **WHEN** control-plane logic observes rising local saturation and a healthier peer is available
- **THEN** it updates routing decisions through host capabilities instead of embedding that policy in the host request path
- **AND** can redirect traffic to a buffer route when the entire cluster is saturated

#### Scenario: Buffered requests replay once local pressure recovers
- **GIVEN** traffic has been redirected to a control-plane buffer route
- **WHEN** the buffer system function observes that local pressure has returned below its replay thresholds
- **THEN** it replays persisted requests to their original mesh routes
- **AND** the host bypasses control-plane overrides for replay-marked requests to avoid infinite buffering loops
