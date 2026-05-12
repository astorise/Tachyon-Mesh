# Proposal: VRAM-Aware Routing and Local Memory Offloading

## Problems

As context windows for AI inference grow, local VRAM (e.g., 24GB on standard RTX nodes) is quickly exhausted, leading to OOM errors. Distributing KV-cache across standard Ethernet networks introduces unacceptable latency that destroys generation speed. Furthermore, the preprocessing step (Feature Flattening) relies on structural depth rather than semantic inlining, leading to inefficient caching.

## What Changes

Instead of a network-bound distributed cache, we will implement intelligent routing and strict local memory tiering.

1. **VRAM-Aware Routing:** The mesh router will consume real-time VRAM telemetry from the nodes. Requests requiring massive context windows will be dynamically routed to nodes with sufficient VRAM headroom, or queued if the cluster is under heavy pressure.
2. **Local CPU-Offloading (PCIe):** Modify the Rust memory allocator to permit KV-cache spilling strictly into local system RAM via the PCIe bus. This avoids network latency penalties while providing a large buffer (e.g., 64GB+ of system RAM) for context overflow.
3. **Semantic Feature Flattening:** Refactor the Feature Flattener module to perform true semantic inlining of AI context markers, maximizing cache hits and reducing the raw memory footprint before offloading is even required.