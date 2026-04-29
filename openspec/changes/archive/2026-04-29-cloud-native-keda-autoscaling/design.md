# Design: KEDA Adapter & Metrics Bridge

## 1. Metrics Source (`system-faas-buffer`)
The buffer FaaS must expose its "backlog depth" (number of pending messages) via a fast, read-only IPC call. 

## 2. External Scaler gRPC (`systems/system-faas-keda/src/lib.rs`)
The module will implement the following KEDA gRPC methods:
- `IsActive`: Returns true if the backlog > 0.
- `GetMetricSpec`: Defines the target threshold (e.g., "Scale up if backlog > 10 jobs per node").
- `GetMetrics`: Returns the current backlog count fetched from the buffer.

## 3. Custom Integration Logic
The `core-host` will act as the "Master Controller":
- It monitors the local hardware health (GPU/NPU usage).
- If hardware is saturated AND the buffer is growing, it sends a `SCALING_REQUIRED` event to the `system-faas-keda` module.
- `system-faas-keda` then reports a high metric value to KEDA to force the creation of a new Kubernetes Pod.

## 4. Minimal Overhead Implementation
- **Zero-Alloc Serialization:** Metrics are passed between modules using the flat-buffers or a simple shared-memory structure to avoid JSON parsing overhead.
- **Poll Frequency:** The adapter only executes its logic during the KEDA poll cycle (default 15s-30s), ensuring it doesn't compete with AI inference for CPU cycles.