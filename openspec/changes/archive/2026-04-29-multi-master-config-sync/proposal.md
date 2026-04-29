# Proposal: Multi-Master Config Sync

## Context
Currently, the architecture assumes a central point of management. However, in a distributed Edge environment, a "Single Master" is a bottleneck and a Single Point of Failure. If the master node is down, the entire mesh becomes unmanaged. We need a way for **Tachyon-UI** to connect to any node and for that node to propagate the new `integrity.lock` and Wasm modules to all peers.

## Proposed Solution
We will implement a **Decentralized Configuration Broadcast**:
1. **Unified Management API:** Every `core-host` exposes the management endpoints (previously reserved for the master) protected by the same admin credentials.
2. **Gossip-Triggered Pull (Efficient Sync):** - When a node (Node A) receives a new config from the UI, it updates its local state and increments a global `config_version`.
   - Node A broadcasts a lightweight `ConfigUpdateEvent { version, checksum, origin_node_id }` via `system-faas-gossip`.
   - Peer nodes receive this event. If the version is newer than their own, they pull the full `integrity.lock` and missing Wasm binaries from Node A via the `system-faas-mesh-overlay` secure P2P tunnel.
3. **Conflict Resolution:** We use a "Highest Version Wins" strategy. All config updates must be signed by an administrative private key to prevent malicious nodes from broadcasting fake configs.

## Objectives
- Achieve high availability for the management plane.
- Minimize bandwidth usage: only the "Notification" is pushed (Gossip), the "Data" is pulled only by nodes that need it.
- Ensure any node can be replaced or rebooted without losing the ability to manage the cluster.