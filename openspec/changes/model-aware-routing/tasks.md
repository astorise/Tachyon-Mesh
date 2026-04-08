# Tasks: Change 047 Implementation

**Agent Instruction:** Implement VRAM state tracking and model affinity routing. Use 4-space indentation for code examples within your implementation notes.

## [TASK-1] Track Hot Models in the Host
1. Update the `ModelManager` (Change 042/044) to maintain a thread-safe `HashSet<String>` of currently loaded model aliases per hardware device.
2. Expose this `HashSet` through the updated `wasi:tachyon/telemetry` WIT interface.

## [TASK-2] Update Gossip Broadcast
1. Modify `system-faas-gossip.wasm` to read the `hot-models` array from the telemetry.
2. Add this array to the compact UDP/HTTP3 gossip payload.
3. Update the in-memory ArcSwap routing table to index remote peers not just by IP, but by `(IP, List<Hot_Models>)`.

## [TASK-3] Enforce Affinity in the Router
1. In the `core-host` HTTP dispatcher, when resolving a target, extract the requested model alias from the HTTP headers or the WASI-NN hook context.
2. When evaluating remote overflow candidates (P2C algorithm), filter the list of peers: discard any peer that does not list the requested model alias in its `hot-models` list.
3. If the filtered list is empty, force local execution regardless of local queue depth.