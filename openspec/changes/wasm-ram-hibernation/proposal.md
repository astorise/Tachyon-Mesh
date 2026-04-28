# Proposal: Wasm RAM Hibernation (Scale-to-Zero)

## Context
Tachyon Mesh relies on Instance Pooling to achieve sub-millisecond FaaS execution. While this provides exceptional performance, it requires keeping Wasm linear memories pre-allocated in the host's RAM. On Edge devices (e.g., IoT gateways, embedded systems) with limited RAM (1GB - 4GB), keeping infrequently used modules "warm" leads to rapid memory exhaustion. We need a mechanism to reclaim RAM from idle instances without forcing them to undergo a full, expensive recompilation (Cold Start) on their next invocation.

## Proposed Solution
We will implement an **Idle Hibernation Engine (Scale-to-Zero)**:
1. **Activity Tracking:** The `core-host` will track the `last_accessed` timestamp for each warm Wasm instance in the pool.
2. **Snapshot / Freeze:** A background garbage collector (the Hibernation Manager) will scan for instances idle for more than `5 minutes`. It will read the Wasm linear memory, serialize it to the host's persistent storage (NVMe/SSD), and drop the instance from RAM.
3. **Restore / Thaw:** When a new HTTP/3 or IPC request arrives for a hibernated module, the host will read the snapshot from disk, allocate a new memory block, copy the state back in, and resume execution. This "Thaw" process is significantly faster than a full JIT compilation Cold Start.

## Objectives
- Achieve true Scale-to-Zero footprint for FaaS modules.
- Maximize tenant density: allow 10,000+ deployed functions on a device with only 2GB of RAM, provided only a subset is active concurrently.
- Transparent execution: the hibernation lifecycle must be completely invisible to the client (other than a slight latency bump on the wake-up request).