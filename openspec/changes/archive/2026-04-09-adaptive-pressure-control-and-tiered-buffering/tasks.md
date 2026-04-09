# Tasks: Change 039 Implementation

**Agent Instruction:** Implement adaptive monitoring and tiered buffering. Do not use nested code blocks in your outputs.

## [TASK-1] Adaptive Monitor Logic
- [x] In the metrics collector, implement a check for the number of active peers. If zero, sleep the thread indefinitely.
- [x] Implement a "Cheap Check" using atomic counters (active_invocations). Only call sysinfo if counters exceed a pre-defined "Caution" threshold.

## [TASK-2] Tiered Buffer Implementation
- [x] Create a Queue Manager that manages a FixedSize RAM buffer and a Disk-backed directory.
- [x] In the Router, instead of returning an error when saturated, call the Queue Manager to store the request.
- [x] Implement the re-injection loop: when CPU pressure subsides, pull requests from RAM first, then Disk, and process them.

## [TASK-3] Cluster Stability (P2C)
- [x] Update the Peer Registry to store a "Last Pressure Update" timestamp and use Hysteresis logic for state transitions.
- [x] Implement the "Power of Two Choices" selection in the outbound load balancer to distribute overflow traffic.

## Validation Step
- [x] Run a single Tachyon node and verify that no monitoring threads are consuming CPU.
- [x] Start a cluster, saturate Node A, and verify it uses RAM buffering before overflowing to Node B.
- [x] Disable Node B, saturate Node A beyond its RAM buffer, and verify that requests are written to Disk and processed later when the load drops.
