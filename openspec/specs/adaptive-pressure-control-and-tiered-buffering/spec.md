# Adaptive Pressure Control and Tiered Buffering

## Purpose
Define how Tachyon minimizes idle monitoring overhead, buffers overload through bounded RAM and disk tiers, and dampens peer selection with pressure-aware routing metadata.

## Requirements
### Requirement: Pressure control adapts monitoring and buffers overload through memory and disk tiers
The host SHALL minimize monitoring overhead when no peers are available and SHALL buffer overload through bounded RAM and disk spillover before failing requests when overflow is unavailable.

#### Scenario: Local pressure rises with no remote overflow target
- **WHEN** the node detects high local pressure and no healthy peer can accept overflow traffic
- **THEN** it reduces unnecessary monitoring work on single-node deployments
- **AND** queues requests through RAM first, then disk spillover, before rejecting additional load

#### Scenario: Buffered requests re-enter execution after pressure subsides
- **WHEN** a saturated route regains available concurrency
- **THEN** buffered requests are replayed in FIFO order with RAM entries drained before disk spillover
- **AND** successful responses expose `x-tachyon-buffered` with the tier that absorbed the request

#### Scenario: Peer pressure metadata avoids oscillation
- **WHEN** multiple eligible peers are available for fast-path delivery
- **THEN** stale peer pressure metadata is ignored
- **AND** the selector samples two candidates and chooses the lower-pressure peer
- **AND** saturated peers do not immediately advertise recovery until they cool below the lower hysteresis threshold
