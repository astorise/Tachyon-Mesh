# Proposal: Resource-Aware Admission Control and Offloading

## Context
To ensure the stability of an Edge node, it is crucial to reject workloads (FaaS) that exceed its physical RAM capabilities. Currently, unexpected heavy loads can trigger an Out-Of-Memory (OOM) system crash. Tachyon Mesh must integrate an admission logic capable of gracefully rejecting requests or seamlessly redirecting them to the broader cluster.

## Objective
1. Add strict resource validation (RAM/VRAM) prior to instantiating any Wasm module.
2. Implement request offloading to other available nodes via the Mesh Overlay if local resources are insufficient.
3. Provide clear visual feedback (UI alerts) and protocol-level feedback (HTTP 503) during node saturation.

## Scope
- Update the FaaS deployment manifest to support a `min_ram_gb` constraint.
- Modify the routing middleware within `core-host` to verify local telemetry before allowing execution.
- Integrate a "System Pressure" notification mechanism into `tachyon-ui` for asynchronous tasks.