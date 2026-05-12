# Specification: VRAM-Aware Routing & Local Offloading

## 1. Local Tiering (CPU-Offloading)
The memory governor must gracefully handle VRAM exhaustion using local system RAM, avoiding disk swap entirely.
* **Allocator Modification:** In `memory_governor.rs`, implement a tiered allocation strategy for the KV-cache tensors. 
* **Threshold:** When requesting memory for KV-cache, check current local VRAM limits. If VRAM allocation exceeds 90%, map new tensor shards to pinned host memory (system RAM) via PCIe. 
* **Constraint:** Do NOT implement disk/NVMe swapping due to the sequential read latency constraints of inference generation.

## 2. VRAM-Aware Load Balancing
The L7 router must be memory-intelligent to prevent node crashes.
* **Metric Ingestion:** The router must query the `telemetry::TelemetrySnapshot` for the `vram_utilization` metric of all candidate nodes in the execution mesh.
* **Routing Decision:** Assign a negative weight penalty to nodes with VRAM utilization > 80%. 
* **Queuing:** If all available nodes are > 90% utilized, place the incoming inference request into a bounded local `await` queue rather than failing immediately with an HTTP 429 or triggering an OOM kill.

## 3. Semantic Inlining Correction
* **Implementation:** The `Feature Flattener` must stop traversing generic JSON structures. It must explicitly identify semantic markers (e.g., conversational turn IDs, system prompt boundaries) and inline these directly into the tokenized chunk metadata. 
* **Goal:** This ensures that contiguous logical sequences have adjacent keys in the KV-cache, drastically improving cache hit rates across dynamic conversations and reducing overall memory waste.