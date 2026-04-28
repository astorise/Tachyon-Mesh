## ADDED Requirements

### Requirement: Rate limiter tracks per-IP state in a strictly bounded LRU cache
The `core-host` rate limiter (`core-host/src/rate_limit.rs`) SHALL store per-IP counters in a strictly bounded LRU cache with a hard maximum number of entries (default `100,000`).

#### Scenario: Cache stays within its bound under spoofed source IPs
- **WHEN** the host receives traffic from more unique source IPs than the configured cache bound
- **THEN** the cache size never exceeds the configured maximum
- **AND** the host's memory footprint for rate limiting remains effectively constant
- **AND** the host does not crash with an out-of-memory error attributable to rate-limiter state

### Requirement: Least-recently-used IPs are evicted on capacity pressure
When the bounded LRU cache reaches capacity, the rate limiter SHALL evict the least recently active IP entries to make room for new ones, while preserving `O(1)` lookup and update for active traffic.

#### Scenario: Active legitimate traffic is preserved under spoofing pressure
- **WHEN** a legitimate client is actively making requests
- **AND** the host is simultaneously receiving traffic from a flood of new spoofed source IPs
- **THEN** the legitimate client's entry is refreshed on each request and is not evicted
- **AND** lookup and update operations for the legitimate client remain `O(1)` in time
- **AND** the spoofed entries occupy only the configured bounded slice of memory before being evicted in turn
