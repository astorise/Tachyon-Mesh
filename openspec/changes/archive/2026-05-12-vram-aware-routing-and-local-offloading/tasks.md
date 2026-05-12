# Tasks

## 1. Core Host (Memory & Routing)
- [x] Update `core-host/src/memory_governor.rs` to implement PCIe CPU-offloading for KV-cache tensors, allowing seamless fallback to pinned host RAM when VRAM hits critical thresholds.
- [x] Modify `core-host/src/host_core/app_runtime.rs` (Routing logic) to consume VRAM metrics from `TelemetrySnapshot` and weigh routing decisions based on available memory headroom.
- [x] Implement a bounded queuing mechanism in the router for giant context requests when the entire cluster's VRAM pool is >90% saturated.

## 2. AI Inference Pipeline
- [x] Refactor the `Feature Flattener` in `core-host/src/ai_inference.rs` to replace structural JSON depth traversal with strict semantic inlining of context markers.

## 3. UI & Observability
- [x] Ensure VRAM utilization vs. System RAM offloading metrics are exposed via the existing MCP telemetry endpoints.
- [x] Update `tachyon-ui/src/components/domains/TachyonAIPanel.ts` to display "VRAM Usage" and a "RAM Offload Active" warning indicator on the node detail view.