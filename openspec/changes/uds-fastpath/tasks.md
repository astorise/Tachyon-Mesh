# Tasks: Change 037 Implementation

**Agent Instruction:** Read `proposal.md` and the delta spec under `specs/`. Implement UDS discovery and fast-path transport in the Rust host.

- [ ] Register a local Unix domain socket endpoint and metadata file for each host instance at startup.
- [ ] Add peer discovery that tracks IP-to-socket mappings from the shared discovery directory and removes stale peers.
- [ ] Prefer `UnixStream` for local mesh connections and fall back to TCP when the fast path is unavailable.
- [ ] Validate local fast-path traffic, stale-peer fallback, and the expected latency improvement over loopback TCP.
