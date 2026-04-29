# Design: Overlay Network and Routing

## 1. The Overlay FaaS (`systems/system-faas-mesh-overlay`)
This new Wasm module manages the heavy lifting of the P2P network.
- **Protocol:** It will use a robust P2P library compiled to Wasm (or rely on host-provided networking WASI extensions) to establish connections using mTLS or the Noise Protocol framework.
- **Heartbeat Payload (JSON/MessagePack):**
  ```json
  {
    "node_id": "edge-gateway-alpha",
    "status": "online",
    "hardware": {
      "gpu": { "present": true, "load_percent": 15 },
      "ram": { "free_mb": 4096 }
    },
    "cached_modules": ["ai-inference", "tde-crypto"]
  }
  ```

## 2. Core-Host Integration (`core-host/src/dispatcher.rs`)
The host routing logic must support a `RemoteFallback` mechanism.

- **Step 1:** Request arrives for `/api/generate`.
- **Step 2:** Host checks local hardware/buffer. If `local_capacity == EXHAUSTED`:
- **Step 3:** Host calls IPC `mesh_overlay.get_best_peer(requirements)`.
- **Step 4:** If a peer is returned (e.g., `edge-gateway-beta`), the host wraps the HTTP request and sends it via IPC to `mesh_overlay.forward_request(peer, payload)`.
- **Step 5:** The Overlay FaaS securely tunnels the request to the peer, waits for the result, and returns it to the host.

## 3. Opt-In Configuration (`integrity.lock`)
The overlay network must be explicitly enabled and configured with bootstrap nodes or discovery multicast settings.
```json
"system_modules": {
  "mesh_overlay": {
    "enabled": true,
    "bootstrap_peers": ["192.168.1.10:8443", "192.168.1.11:8443"]
  }
}
```