# Tasks: Change 059 Implementation

**Agent Instruction:** Implement capability broadcasting and filtering. Use bitflags if possible for maximum performance during the filtering phase, as string matching on every request adds routing overhead. Use 4-space indentation.

## [TASK-1] Compile-Time Capability Auto-Detection
1. In `core-host`, create a `Capabilities` struct or bitmask.
2. Use conditional compilation (`#[cfg(feature = "gpu-candle")]`, `#[cfg(target_os = "linux")]`) to automatically populate this struct at startup.
3. If the node runs in a V1 Docker container, ensure the `legacy:oci` flag is enabled.

## [TASK-2] Update the Gossip FaaS
1. Modify the `system-faas-gossip.wasm` state struct to include the new capability list.
2. Ensure that when a new peer joins the cluster, its capabilities are permanently cached in the local routing table (since capabilities rarely change without a node restart, unlike CPU load).

## [TASK-3] The Routing Dispatcher
1. Update the Target definition parser in `integrity.lock` to parse the `requires` array.
2. In the HTTP/TCP dispatcher, before selecting a peer for overflow, perform a bitwise `AND` (or subset check) between the Target's requirements and the Peer's capabilities.
3. If the local node receives a request it cannot handle, and no capable peers exist in the Mesh, immediately return a `503 Service Unavailable` with a clear "Missing Capability" error message, rather than hanging or crashing.