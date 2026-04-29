# Proposal: P2P Mesh Overlay & Service Discovery

## Context
Currently, Tachyon Mesh nodes operate as isolated islands. If Node A receives an AI inference request but its GPU is saturated, it must queue the request or drop it, even if Node B (on the same local network or connected via the internet) is sitting idle with a free GPU. While `system-faas-gossip` exists, it is designed for lightweight state synchronization (like CRDTs), not for establishing secure data tunnels or making complex routing decisions.

## Proposed Solution
We will implement an entirely new, optional System FaaS dedicated to advanced networking: `system-faas-mesh-overlay`.
1. **Service Discovery:** This module will broadcast a "Hardware Heartbeat" containing the node's capabilities (e.g., `gpu_available: true`, `active_faas_count: 5`, `supported_models: ["llama3"]`).
2. **Dynamic Routing Table:** It will maintain a real-time routing table of all peers in the mesh and their current hardware load.
3. **Secure Tunneling (mTLS/Noise):** It will establish a secure, Zero-Trust peer-to-peer overlay network.
4. **Core-Host Delegation:** If the `core-host` cannot fulfill a request locally (e.g., GPU full), it will ask `system-faas-mesh-overlay` for the best peer, forward the raw request payload through the secure tunnel, and stream the remote response back to the client as if it were processed locally.

## Objectives
- Maximize hardware utilization across the entire Edge deployment.
- Maintain a strictly modular architecture by keeping this heavy networking logic out of the baseline `system-faas-gossip`.
- Enable transparent cross-node FaaS execution without a centralized Load Balancer.