# Implementation Tasks

## Phase 1: Bootstrap the Overlay FaaS
- [ ] Create the new crate `systems/system-faas-mesh-overlay`.
- [ ] Implement the Service Discovery loop (broadcasting and receiving hardware heartbeats).
- [ ] Implement the in-memory Routing Table to track peer capabilities and health.

## Phase 2: Secure Tunneling
- [ ] Implement a secure transport layer (mTLS) for peer-to-peer communication.
- [ ] Expose an IPC endpoint `forward_request` that allows the host to send arbitrary byte payloads to a specific peer ID over the secure tunnel.

## Phase 3: Host Routing Delegation
- [ ] In `core-host`, update the execution dispatcher to catch `ResourceExhausted` errors (e.g., from the GPU lock).
- [ ] Implement the fallback logic: query the overlay FaaS for a peer, and if available, delegate the request execution remotely.

## Phase 4: Validation
- [ ] **Test Load Balancing:** Spin up Node A (without GPU) and Node B (with GPU). Send an AI inference request to Node A. 
- [ ] Verify that Node A transparently forwards the request to Node B via the `mesh-overlay`, and Node A's client receives the correct generated text.
- [ ] Verify that if the `mesh_overlay` is disabled in `integrity.lock`, Node A immediately returns a `503 Service Unavailable` without attempting discovery.