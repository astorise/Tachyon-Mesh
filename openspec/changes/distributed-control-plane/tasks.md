# Tasks: Change 038 Implementation

**Agent Instruction:** Read the proposal.md and specs.md. Implement the Control Plane System FaaS and the Host Telemetry interface. Do not use nested code blocks in your outputs.

## [TASK-1] Host Telemetry and Dynamic Routing API
- [ ] In the core-host, add atomic counters for active instances and memory pages allocated.
- [ ] Define and implement the tachyon:telemetry/metrics WIT interface to return these counters as percentages.
- [ ] Expose a host function tachyon:routing/update-target allowing a WASM module to safely mutate the host's ArcSwap routing table.

## [TASK-2] Implement the Gossip System FaaS
- [ ] Create a WASM component named system-faas-gossip.
- [ ] Implement a loop that queries the local host telemetry.
- [ ] Implement a lightweight network exchange to share pressure scores with discovered peers.
- [ ] Implement the "Power of Two Choices" algorithm. When local pressure exceeds the soft limit, determine the optimal peer and call the routing update host function to redirect traffic.
- [ ] Implement hysteresis logic to restore local routing only when pressure drops significantly.

## [TASK-3] Implement the Buffer System FaaS
- [ ] Create a WASM component named system-faas-buffer.
- [ ] Implement an HTTP handler that receives requests, categorizes their priority, and writes the serialized request to an in-memory queue.
- [ ] If the memory queue exceeds a configured threshold, write the serialized request to the filesystem using standard WASI I/O.
- [ ] Implement a scheduled loop that checks cluster telemetry and replays stored requests to their original target routes when resources become available.

## Validation Step
- [ ] Deploy the core-host with the gossip and buffer FaaS components active.
- [ ] Use a load-testing tool to saturate the local host.
- [ ] Verify that the gossip FaaS successfully updates the routing table, causing the core-host to forward traffic to a secondary node.
- [ ] Saturate all available nodes in the cluster.
- [ ] Verify that traffic is successfully routed to the buffer FaaS, written to disk, and automatically replayed once the load testing tool is stopped.
