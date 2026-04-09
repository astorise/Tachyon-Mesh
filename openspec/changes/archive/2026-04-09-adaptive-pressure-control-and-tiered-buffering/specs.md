# Specifications: Adaptive Control & Buffering

## 1. Single-Node Deactivation
Upon startup and peer discovery cycles, if `peer_count == 0`:
- Set `monitor_interval = Infinite`.
- Short-circuit all overflow logic branches in the Router.

## 2. Tiered Wait Queue (Pressure Valve)
If `LocalPressure > HardLimit` AND `NoRemoteCapacity`:
- **Stage 1 (RAM):** Push request to `internal_memory_buffer`. Set `X-Tachyon-Buffered: RAM` header.
- **Stage 2 (Disk):** If `internal_memory_buffer` is full, serialize request to `/var/lib/tachyon/spillover/`. Set `X-Tachyon-Buffered: Disk`.
- **Worker:** A background task polls the CPU pressure. When `Pressure < 80%`, it re-injects buffered requests into the execution pipeline.

## 3. Load Dampening (Anti-Oscillation)
- **Hysteresis:** A node marked as "Saturated" at 90% load only returns to "Healthy" status when load drops below 80%.
- **P2C Selection:** The Router picks two random peers from the registry and selects the one with the lowest reported pressure to avoid the "Thundering Herd" on a single least-loaded node.