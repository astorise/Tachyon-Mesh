# Proposal: Cloud-Native Auto-Scaling (KEDA Integration)

## Context
Tachyon Mesh manages local concurrency through its internal buffer and instance pooling. However, when the global workload exceeds the physical capacity of a node (CPU/GPU/RAM), we must trigger the creation of new Mesh nodes at the Kubernetes level. Currently, K8s HPA (Horizontal Pod Autoscaler) based on CPU/RAM is too slow and "reactive". We need **Event-Driven Scaling** based on the actual number of pending AI jobs or FaaS requests.

## Proposed Solution
We will implement an **External Scaler adapter** using `system-faas-keda`:
1. **Integration with Custom Scaler:** The `core-host` already has a custom internal logic for worker management. We will extend this to export "Scaling Signals" to the `system-faas-keda` module via a high-speed IPC channel.
2. **KEDA External Scaler Protocol:** The `system-faas-keda` module will implement the gRPC External Scaler interface required by KEDA.
3. **Minimal Overhead:** Instead of a continuous push, the adapter will only read the shared metrics state (from `system-faas-buffer` and `core-host`) when polled by the KEDA operator (Pull-based). This avoids any impact on the critical path of inference or routing.

## Objectives
- Seamlessly scale Tachyon Mesh nodes based on real-time buffer backlog.
- Integrate horizontal scaling (K8s) with vertical scaling (local workers) for a unified "Custom Auto-scaling" experience.
- Maintain a sub-microsecond overhead on the core routing logic.