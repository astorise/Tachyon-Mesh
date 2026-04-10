# Tasks: Change 056 Implementation

**Agent Instruction:** Implement Control-Plane overflow for L4 bridging. The User FaaS must never know whether the bridge was allocated locally or remotely; the abstraction must be seamless. Use 4-space indentation.

## [TASK-1] Host L4 Telemetry
- [x] Update the `BridgeManager` (Change 055) in the `core-host` to track the total number of active relay tasks and the estimated bytes/sec throughput.
- [x] Expose these metrics via the `wasi:tachyon/telemetry` interface.
- [x] Update `system-faas-gossip.wasm` to broadcast this `l4_load_score` to the cluster.

## [TASK-2] Update Bridge Controller
- [x] Modify `system-faas-bridge.wasm`. When `create-bridge` is called, evaluate the local `l4_load_score`.
- [x] Implement the threshold logic. If the local node is saturated, pick the best peer from the Gossip routing table.
- [x] If delegating, use the internal Mesh client to send an HTTP POST to `https://<peer-ip>/system-bridge/create`.

## [TASK-3] Return Endpoint Abstraction
- [x] Read the new `advertise_ip` from the host configuration.
- [x] Ensure the Host FFI function returns this IP along with the local ports.
- [x] Update the User FaaS example (e.g., the SIP router) to inject this returned IP into its SIP/SDP response body, instructing external clients to connect directly to the designated node.
