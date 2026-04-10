# Specifications: Dynamic Bridging & System FaaS Controller

## 1. The Bridge WIT Interface (`tachyon:mesh/bridge-controller`)
User and system guests use the same typed bridge API.

- Define `bridge-config { client-a-addr, client-b-addr, timeout-seconds }`.
- Define `bridge-endpoints { bridge-id, port-a, port-b }`.
- Import `bridge-controller` into `faas-guest` and `system-faas-guest`.

## 2. Host-Level Data Plane (The Relay)
When a bridge is created:

- The `core-host` allocates two ephemeral UDP sockets on loopback.
- It spawns a dedicated Tokio relay task that forwards datagrams in both directions.
- The relay exits on inactivity timeout or explicit teardown.
- Packet forwarding stays in host memory and does not invoke WASM per packet.

## 3. Control Plane Split
The bridge control path stays in WASM while the packet path stays native.

- User FaaS call the host `bridge-controller` import.
- For user routes, the host forwards the request to the sealed `/system/bridge` route.
- For the privileged `/system/bridge` route, the host services the bridge request directly against the shared `BridgeManager`.
- `system-faas-bridge` persists bridge metadata into its writable RAM volume so operators can inspect active sessions.
