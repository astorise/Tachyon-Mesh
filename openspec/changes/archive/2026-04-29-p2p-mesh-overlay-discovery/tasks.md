# Implementation Tasks

## Phase 1: Bootstrap the Overlay FaaS
- [x] Create the new crate `systems/system-faas-mesh-overlay`.
- [x] Implement the Service Discovery loop (broadcasting and receiving hardware heartbeats).
- [x] Implement the in-memory Routing Table to track peer capabilities and health.

## Phase 2: Secure Tunneling
- [x] Implement a secure transport layer (mTLS) for peer-to-peer communication.
- [x] Expose an IPC endpoint `forward_request` that allows the host to send arbitrary byte payloads to a specific peer ID over the secure tunnel.

## Phase 3: Host Routing Delegation
- [x] In `core-host`, update the execution dispatcher to catch `ResourceExhausted` errors (e.g., from the GPU lock).
- [x] Implement the fallback logic: query the overlay FaaS for a peer, and if available, delegate the request execution remotely.

## Phase 4: Validation
- [x] **Test Load Balancing:** Spin up Node A (without GPU) and Node B (with GPU). Send an AI inference request to Node A. 
- [x] Verify that Node A transparently forwards the request to Node B via the `mesh-overlay`, and Node A's client receives the correct generated text.
- [x] Verify that if the `mesh_overlay` is disabled in `integrity.lock`, Node A immediately returns a `503 Service Unavailable` without attempting discovery.
