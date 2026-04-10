# Tasks: Change 055 Implementation

**Agent Instruction:** Implement the dynamic UDP bridge path with a privileged system controller and a user-facing bridge API.

## [TASK-1] Host Dynamic Socket Manager
- [x] Implement a `BridgeManager` in `core-host` that allocates ephemeral UDP bridge ports.
- [x] Bind two local UDP sockets per bridge and spawn a dedicated Tokio relay task for bidirectional forwarding.
- [x] Use `tokio::select!` to enforce inactivity cleanup and support explicit teardown.
- [x] Track active relay count and relayed byte totals inside the manager for future control-plane steering.

## [TASK-2] Privileged System FaaS: `system-faas-bridge`
- [x] Create `system-faas-bridge` as a system WASM component.
- [x] Expose `create-bridge` and `destroy-bridge` over the sealed `/system/bridge` route.
- [x] Persist active bridge session metadata into the component's writable RAM volume under `/sessions`.

## [TASK-3] User FaaS Integration
- [x] Extend `tachyon.wit` with a `bridge-controller` interface and import it into `faas-guest` and `system-faas-guest`.
- [x] Update the Wasmtime host linker so user guests route bridge requests through `/system/bridge`, while the system bridge route receives direct host access to the shared `BridgeManager`.
- [x] Implement `guest-voip-gate` as a sample user FaaS that accepts a start-call payload, allocates a bridge, and returns the assigned ports.

## Validation Step
- [x] Validate the host relay directly with two in-process UDP clients and bidirectional packet forwarding.
- [x] Validate the end-to-end flow by calling `/api/voip-gate`, allocating a bridge through `/system/bridge`, and relaying a UDP payload from client A to client B.
- [x] Build the new WASM artifacts (`guest-voip-gate`, `system-faas-bridge`) locally and in CI.
- [x] Run targeted tests for the new components plus full workspace `clippy`, `test`, and `build`.
