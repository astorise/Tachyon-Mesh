# Design: Adaptive Pressure Control & Tiered Buffering

## Summary
The host now degrades overload locally before dropping requests. Saturated HTTP routes spill requests into a bounded queue manager that keeps the hot path in memory first, then serializes overflow to disk. A replay worker reinjects buffered work once route pressure subsides.

## Monitoring Strategy
- Peer pressure publication stays dormant on single-node deployments by sleeping the monitor loop when no peers are discovered.
- Pressure classification uses cheap in-process counters first: active requests and pending route waiters.
- State transitions use hysteresis so a saturated node must cool below the lower threshold before it advertises recovery.

## Buffering Strategy
- Buffered requests are serialized as route path, selected module, request metadata, payload, hop limit, and trace context.
- The queue manager keeps a bounded RAM deque, then spills additional requests into a spool directory adjacent to the manifest.
- Replayed responses preserve normal route execution semantics and tag successful replies with `x-tachyon-buffered: ram|disk`.

## Peer Selection
- UDS peer metadata now carries pressure state and last-update timestamps.
- Fast-path peer discovery ignores stale pressure metadata and uses a power-of-two-choices pick across matching peers to avoid herd behavior on a single candidate.

## Validation
- Added host tests for RAM-to-disk spill and saturated-route buffering timeout.
- Full `core-host` tests, workspace `clippy`, `cargo build`, and `openspec validate --all` pass with the new buffering path enabled.
