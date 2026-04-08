# Proposal: Change 059 - Heterogeneous Mesh & Capability Routing

## Context
As Tachyon evolves from V1 (Dockerized Linux) to V2 (Multi-platform native binaries via Cargo Features), the cluster becomes heterogeneous. A node compiled for Windows cannot run OCI legacy containers, and a Raspberry Pi cannot execute CUDA workloads. If the Mesh router assigns a task to an incapable node, the execution will fail, causing latency spikes and retry loops.

## Objective
1. Nodes must auto-detect and declare their capabilities (Hardware, OS, enabled Features) at startup.
2. The `system-faas-gossip` must broadcast these capabilities alongside the real-time load metrics.
3. The Mesh Router must implement a "Filter-Then-Score" algorithm: filter out nodes lacking required capabilities before scoring them for latency or load.

## Scope
- Update the `core-host` initialization to build a `NodeCapability` bitmask or tag list based on Rust `#[cfg]` flags.
- Update the Gossip protocol payload.
- Update the routing dispatcher to match Target requirements against Node capabilities.