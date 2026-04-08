# Proposal: Change 038 - Distributed Control Plane (Gossip, Overflow & Buffering)

## Context
A true distributed runtime must handle local resource saturation (CPU/RAM limits) by overflowing traffic to healthy peer nodes, or buffering it if the entire cluster is saturated. However, embedding complex logic like Gossip protocols, the "Power of Two Choices" (P2C) load-balancing algorithm, and disk-spilling queues directly into the Rust `core-host` violates our "Zero-Overhead" and microkernel philosophy. We must separate the Data Plane (the Rust host) from the Control Plane (System FaaS).

## Objective
1. Keep the `core-host` strictly as a Data Plane: it only reads an in-memory routing table and executes WASM or forwards bytes.
2. Introduce a new WIT capability: `tachyon:telemetry/metrics`, allowing the host to expose its atomic hardware counters to System FaaS.
3. Build the Control Plane using System FaaS: `system-faas-gossip` (evaluates cluster pressure and rewrites local KVS routing tables dynamically) and `system-faas-buffer` (handles RAM/Disk queues during total cluster saturation).

## Scope
- Implement the `tachyon:telemetry` interface in the Rust host.
- Provide a host capability for System FaaS to atomically update the host's route targets (RCU pattern).
- Develop `system-faas-gossip.wasm` to poll local telemetry, gossip with peers, and dynamically redirect routes to remote IPs when local pressure is high.
- Develop `system-faas-buffer.wasm` to safely store and replay requests when no node has capacity.

## Success Metrics
- Zero monitoring overhead on a single-node deployment (by simply omitting the System FaaS from the manifest).
- When Node A hits 85% CPU, its `system-faas-gossip` dynamically alters the internal routing table to point to Node B. The Rust host forwards the traffic blindly, unaware of the "overflow" concept.
- When the entire cluster hits 95% CPU, traffic is seamlessly routed to `system-faas-buffer`, preventing HTTP 503 drops.