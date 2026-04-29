# Implementation Tasks

## Phase 1: Configuration & Hook
- [x] Add the `sync_to_cloud` boolean to the relevant storage manifests in `integrity.lock`.
- [ ] Update the `core-host` storage driver to emit an async event upon successful writes to flagged resources.

## Phase 2: CDC FaaS Local Spooling
- [x] Bootstrap `systems/system-faas-cdc`.
- [x] Implement a persistent queue for the events (using a local file or SQLite) to ensure no data is lost if the node crashes before syncing.

## Phase 3: Cloud Replication Engine
- [x] Implement the background draining loop in `system-faas-cdc`.
- [x] Add connectivity detection (e.g., pinging the upstream health endpoint).
- [x] Implement exponential backoff for failed cloud upload attempts.

## Phase 4: Validation
- [ ] **Offline Test:** Disconnect the machine from the internet. Write data to the `sync_to_cloud` enabled FaaS. Verify local reads are instant and the CDC local spool grows.
- [ ] **Sync Test:** Reconnect the internet. Verify the CDC module detects the connection, pushes the data to the mock Cloud endpoint, and clears its local spool.
