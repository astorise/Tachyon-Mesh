# Tasks: Change 055 Implementation

**Agent Instruction:** Implement the dynamic UDP bridging mechanism. Focus on the separation between the privileged System FaaS and the Host-level relay tasks. Use 4-space indentation for code examples.

## [TASK-1] Host Dynamic Socket Manager
- [ ] Implement a `BridgeManager` struct in Rust that manages a pool of ephemeral UDP ports.
- [ ] Add a method `create_relay(addr_a, addr_b)` that:
    - Binds two local `UdpSocket`.
    - Spawns a `tokio::spawn` task for bidirectional forwarding.
    - Uses `tokio::select!` to handle timeouts and manual termination.

## [TASK-2] Privileged System FaaS: `system-faas-bridge`
- [ ] Create a new System FaaS that acts as the "Network Controller".
- [ ] This FaaS exposes the `create-bridge` logic to the Mesh.
- [ ] It keeps a mapping of active sessions in its own RAM volume (Change 033) to allow for monitoring and manual teardown.

## [TASK-3] User FaaS Integration
- [ ] Update the Wasmtime Linker to allow User FaaS (like a SIP router) to call the `system-faas-bridge` via the internal Mesh (Change 029/037).
- [ ] Implement a sample "VoIP-Gate" FaaS that:
    - Receives a "Start Call" command.
    - Calls `system-faas-bridge` to get media ports.
    - Returns the assigned ports to the caller.

## Validation Step
- [ ] Launch two UDP clients (Netcat/Socat).
- [ ] Trigger a User FaaS to create a bridge between Client A and Client B.
- [ ] Send 1000 UDP packets from A to the assigned Bridge Port.
- [ ] Verify that Client B receives all packets.
- [ ] Monitor the Host CPU: verify that `wasmtime` execution time does not increase during the packet transfer.
