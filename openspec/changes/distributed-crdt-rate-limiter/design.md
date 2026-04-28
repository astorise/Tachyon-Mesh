# Design: CRDT & Gossip Integration

## 1. Schema Update (`core-host/src/main.rs`)
Add the optional flag to the route configuration.

```rust
pub struct IntegrityRoute {
    // ... existing fields
    #[serde(default)]
    pub distributed_rate_limit: bool,
}
```

## 2. The Limiter System FaaS (`systems/system-faas-dist-limiter`)
This module manages the global state:
- **State:** A map of `(IP, TimeWindow) -> CRDT_Counter`.
- **Sync:** Periodically (e.g., every 500ms), it serializes its delta state and sends it to `system-faas-gossip` for broadcast.
- **Merge:** When receiving a gossip payload, it merges the remote counters into its local state using CRDT join semantics (max value for counters).

## 3. Core Host Middleware (`core-host/src/rate_limit.rs`)
Update the rate limiting logic to support the distributed check:

- **Local Check (Always):** Consult the local LRU cache (fast path).
- **Distributed Check (Conditional):** - If `route.distributed_rate_limit == true`:
    - Dispatch an IPC call to `system-faas-dist-limiter.check(ip)`.
    - If result is `Denied`, return `429 Too Many Requests`.
    - Else, proceed.