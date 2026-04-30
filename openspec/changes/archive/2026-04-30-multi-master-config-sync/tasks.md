# Implementation Tasks

## Phase 1: API & Versioning
- [x] Add `version` and `signature` fields to the `integrity.lock` parser.
- [x] Ensure the management API is enabled on all nodes by default (secured by mTLS/Auth).

## Phase 2: Gossip Integration
- [x] Create a new gossip topic `tachyon.config.sync`.
- [x] Implement the broadcast logic in `core-host` when a file write to `integrity.lock` is detected.

## Phase 3: P2P Pull Logic
- [x] Implement a "File Provider" in `system-faas-mesh-overlay` that can serve the current `integrity.lock` and modules to peers.
- [x] Implement the "Puller" logic: when a new version is detected, the node fetches the manifest via P2P.

## Phase 4: Validation
- [x] **Multi-Node Test:** Start 3 nodes. Connect the UI to Node 3. Update a FaaS route.
- [x] Verify that Node 1 and Node 2 correctly receive the notification and update their local routes within seconds.
- [x] **Partition Test:** Disconnect Node 2. Update the config via Node 1. Reconnect Node 2 and verify it automatically syncs to the latest version via Gossip reconciliation.
