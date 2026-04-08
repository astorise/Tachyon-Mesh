# Tasks: Change 037 Implementation

**Agent Instruction:** Read `proposal.md` and the delta spec under `specs/`. Implement UDS discovery and fast-path transport in the Rust host.

- [x] Register a local Unix domain socket endpoint and metadata file for each host instance at startup.
- [x] Add peer discovery that tracks IP-to-socket mappings from the shared discovery directory and removes stale peers.
- [x] Prefer `UnixStream` for local mesh connections and fall back to TCP when the fast path is unavailable.
- [x] Validate local fast-path traffic, stale-peer fallback, and the expected latency improvement over loopback TCP.
