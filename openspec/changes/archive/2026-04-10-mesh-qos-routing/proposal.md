# Proposal: Change 046 - Mesh-Aware QoS Routing (Global Pressure Steering)

## Context
In Change 038, we established a Gossip FaaS that redirects traffic when a node's global CPU/RAM is saturated. In Change 045, we introduced hardware-specific queues with local QoS (RealTime, Standard, Batch). We must now unify these concepts. The Mesh router must make overflow decisions not just based on global node health, but on the *Hardware Queue Depth* crossed with the request's *QoS Class*. 

## Objective
1. Extend the Host Telemetry API to report the queue depth of specific hardware accelerators (GPU, NPU, TPU) categorized by QoS.
2. Update the `system-faas-gossip` to broadcast this "Global Hardware Map" across the cluster.
3. Implement Asymmetric Overflow Policies in the Host Router:
   - `Batch` tasks strongly prefer local execution (to save network bandwidth) and only overflow to `system-faas-buffer` if the local queue is critically full.
   - `RealTime` tasks aggressively overflow to remote peers if the local hardware queue has even a slight backlog, minimizing tail latency.

## Scope
- Update `wasi:tachyon/telemetry` to include hardware-specific queue metrics.
- Modify the routing table schema (mutated by the Gossip FaaS) to include QoS-specific route targets.
- Implement the "Cost of Network vs Cost of Queueing" decision matrix in the Rust `core-host` HTTP dispatcher.

## Success Metrics
- A flood of `Batch` GPU requests stays entirely on Node A, queueing locally or buffering to disk, preserving cluster network bandwidth.
- During this flood, a `RealTime` GPU request arriving at Node A is instantly forwarded by the Mesh to Node B (which has an idle GPU), bypassing the local queue entirely.