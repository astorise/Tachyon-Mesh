# Tasks: Change 046 Implementation

**Agent Instruction:** Integrate the QoS logic into the distributed routing mechanism. Ensure the Rust host remains a fast Data Plane, delegating the complex cluster map evaluation to the Gossip FaaS. Do not use nested code blocks.

## [TASK-1] Host Telemetry Granularity
- [x] Update the Rust host's `BatchScheduler` for each hardware type to maintain atomic counters for the depth of each QoS queue tier.
- [x] Implement the updated `wasi:tachyon/telemetry` WIT interface to allow the Gossip FaaS to read these counters.

## [TASK-2] Update the Gossip FaaS Logic
- [x] Modify `system-faas-gossip.wasm` to retrieve the new hardware-specific telemetry.
- [x] Update the gossip protocol payload to include `gpu_rt_load`, `gpu_batch_load`, `npu_rt_load`, etc.
- [x] Implement the decision matrix: compute the most optimal peer for `RealTime` GPU tasks and update the host's routing table (e.g., mapping `target: gpu-bot-rt` to `node-b-ip`). Do not map `Batch` targets to remote peers unless local failure is imminent.

## [TASK-3] Host Router Enforcement
- [x] In the `core-host` HTTP dispatcher, before allocating WASM memory, check the requested FaaS's QoS and primary hardware requirement.
- [x] Evaluate the local queue depth against the Asymmetric Thresholds (e.g., if QoS is RealTime and local GPU is busy, check the ArcSwap routing table for a remote shortcut).
- [x] If the routing table indicates a remote peer for this specific QoS target, use the existing Mesh mTLS client to forward the HTTP request, bypassing local FaaS instantiation completely.

## Validation Step
- [x] Launch Node A and Node B. Both have GPUs.
- [x] Saturate Node A's GPU with 1000 `gpu-batch` requests.
- [x] Observe that Node A keeps all 1000 requests local (queueing them or buffering them to disk).
- [x] Send a single `gpu-live-chat` (RealTime) request to Node A.
- [x] Verify via the network logs that Node A's HTTP router intercepts the request and instantly forwards it to Node B via mTLS, where Node B's GPU executes it immediately.
