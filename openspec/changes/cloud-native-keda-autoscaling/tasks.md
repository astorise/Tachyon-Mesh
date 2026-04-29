# Implementation Tasks

## Phase 1: Metrics Extraction
- [x] Update `system-faas-buffer` to track the current queue depth in a thread-safe, low-latency counter.
- [x] Expose an internal IPC endpoint `get_queue_depth()`.

## Phase 2: KEDA gRPC Implementation
- [x] In `systems/system-faas-keda`, implement the gRPC services required by the KEDA External Scaler specification.
- [x] Link this module to the internal `system-faas-buffer` to retrieve real-time metrics.

## Phase 3: Integration with Custom Scaling
- [ ] Update the `core-host` custom scaling logic to balance between:
    - **Scale Up (Internal):** Spawning more Wasm instances (up to node limit).
    - **Scale Out (Cloud-Native):** Signaling `system-faas-keda` to trigger KEDA for more nodes.

## Phase 4: Validation
- [ ] **Test Scaling:** Deploy Tachyon Mesh on a K8s cluster with KEDA.
- [ ] Flood the system with AI requests until the local GPU/Buffer is saturated.
- [ ] Verify that KEDA correctly triggers the creation of a new Pod (Horizontal Scaling) based on the metrics provided by `system-faas-keda`.
