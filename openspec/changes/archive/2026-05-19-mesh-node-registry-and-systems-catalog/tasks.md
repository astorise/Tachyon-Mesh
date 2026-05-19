## 1. WIT and workspace setup

- [x] 1.1 Inventory every existing `control-plane-faas` guest in the workspace and list them in the change PR description (so reviewers know exactly which crates are rebuilt)
- [x] 1.2 Extend the `control-plane-faas` world in `wit/tachyon.wit` to add `import kv-partition` (single source of truth — no `-with-kv` variant per D4)
- [x] 1.3 Rebuild every guest from the 1.1 inventory in the same commit as 1.2; the import is additive but each guest's generated bindings must be regenerated
- [x] 1.4 Run the workspace test suite to confirm no `control-plane-faas` guest broke; treat any failure as a blocker for this change (rolling back to a `-with-kv` variant is explicitly out of scope)
- [x] 1.5 Create `systems/system-faas-node-registry/` crate (Rust + WASM, control-plane-faas guest) with `Cargo.toml`, `src/lib.rs`, and the `wit_bindgen::generate!` block
- [x] 1.6 Add the new crate to the root `Cargo.toml` workspace members
- [x] 1.7 Author `systems/manifest.toml` with one entry per existing `system-faas-*` crate (`slug`, `crate_name`, `version`, `description`)
- [x] 1.8 Add a `build.rs` check that fails the build when `systems/manifest.toml` references a slug whose crate is missing from the workspace

## 2. Host-side ReDB table and kv-partition mapping

- [x] 2.1 Add a new ReDB table descriptor in `core-host/src/store` for the `node-registry` keyspace
- [x] 2.2 Extend the host's `kv-partition` implementation so a guest-side `table::new("node-registry")` resolves to that ReDB table
- [x] 2.3 Add a unit test that opens the table from inside a fake guest, writes a row, closes the host, reopens, and reads the row back

## 3. Node registry FaaS — data model and persistence

- [x] 3.1 Define `EnrolledNode`, `NodeCapabilities`, `GpuStats`, `ActiveSystem` Rust structs inside `system-faas-node-registry/src/types.rs` with `serde` derives
- [x] 3.2 Implement a `Registry` wrapper that owns a `kv-partition::table` handle for `"node-registry"` and provides `record_approval`, `update_capabilities`, `list`, `get`, `mark_stale_after`
- [x] 3.3 Implement an in-FaaS cache of the parsed list, invalidated on every write, to amortise JSON parsing across read-heavy traffic
- [x] 3.4 Unit tests with a `kv-partition` mock: persistence round-trip, list filtering by status, stale transition, awaiting-capabilities placeholder

## 4. Migrate the enrollment ceremony into the FaaS

- [x] 4.1 Port the PIN generator, the `EnrollmentSession`/`EnrollmentOutcome` types, and the `EnrollmentManager` state machine from `core-host/src/node_enrollment.rs` into `system-faas-node-registry/src/enrollment.rs`; keep behaviour identical
- [x] 4.2 Implement the FaaS `handle-request` export so that POST `/admin/enrollment/start`, POST `/admin/enrollment/approve/{session_id}`, and GET `/admin/enrollment/poll/{session_id}` are served end-to-end inside the component
- [x] 4.3 On successful approval, call `Registry::record_approval` so the new node lands in the persisted set with `status = "awaiting-capabilities"`
- [x] 4.4 In `core-host`, replace `admin_enrollment_start/approve/poll_handler` bodies with forwarders that invoke the FaaS `handle-request` export and stream the response back
- [x] 4.5 Run the existing enrollment-related integration tests; verify they pass with the FaaS in place
- [x] 4.6 Once green, remove `core-host/src/node_enrollment.rs` and any other host-side state that the FaaS now owns
- [x] 4.7 Add an integration test that approves an enrolment end-to-end and asserts the row exists in the ReDB `node-registry` table after a host restart

## 5. Capability reporting + stale sweep

- [x] 5.1 Add the `POST /admin/nodes/{node_id}/capabilities` route in the FaaS `handle-request`; validate the caller's mTLS identity against the stored public key
- [x] 5.2 Update the registry record through `kv-partition::table::set`; transition `status` to `"online"` and refresh `last_seen`
- [x] 5.3 Implement the FaaS `on-tick` export that scans the table and marks any entry whose `last_seen` exceeds the stale threshold as `status = "stale"`
- [x] 5.4 Add the host-side forwarder for `/admin/nodes/{node_id}/capabilities`
- [x] 5.5 Integration test: enrol → POST capabilities → `GET /admin/nodes` returns the node with reported RAM/GPU and `status = "online"`

## 6. Systems catalog inside the FaaS

- [x] 6.1 Generate `static_catalog.rs` from `systems/manifest.toml` at build time (helper in `build.rs`)
- [x] 6.2 Implement `list_registered_systems()` returning the static catalog
- [x] 6.3 Extend `EnrolledNode` (or `NodeCapabilities`) with `active_systems: Vec<ActiveSystem>` carrying `{ slug, version }`
- [x] 6.4 Implement `list_deployed_systems()` by aggregating `active_systems` across all rows; compute `has_drift` per system by comparing per-node versions against the catalog version
- [x] 6.5 Expose both as HTTP routes (`GET /admin/systems/registered`, `GET /admin/systems/deployed`) routed via the FaaS handler
- [x] 6.6 Unit tests: empty deployed list, multi-node aggregation, version-drift detection

## 7. tachyon-client surface

- [x] 7.1 Add Rust types in `tachyon-client/src/lib.rs`: `EnrolledNode`, `NodeCapabilities`, `GpuStats`, `RegisteredSystem`, `DeployedSystem`, `ClusterHardwareSummary`
- [x] 7.2 Add async functions: `list_enrolled_nodes()`, `get_node_capabilities(node_id: &str)`, `list_registered_systems()`, `list_deployed_systems()`, `get_cluster_hardware_summary()`
- [x] 7.3 Each function hits the new admin routes through the existing authenticated transport; provide an offline fallback that returns an empty vector with `source = "offline"`

## 8. Tauri commands

- [x] 8.1 Add `#[tauri::command]` entries in `tachyon-ui/src/main.rs`: `list_enrolled_nodes`, `get_node_capabilities`, `list_registered_systems`, `list_deployed_systems`, `get_cluster_hardware_summary`
- [x] 8.2 Register the new handlers in the Tauri `invoke_handler` builder
- [x] 8.3 Regenerate the JSON schemas under `tachyon-ui/gen/schemas/` for the new payloads

## 9. UI — Nodes view

- [x] 9.1 Create `tachyon-ui/src/components/domains/TachyonNodesPanel.ts` extending `TachyonConfigDashboard`, mounting under `<tachyon-nodes-panel>`
- [x] 9.2 Fetch on connect: `list_enrolled_nodes` (table) and `get_cluster_hardware_summary` (header KPIs)
- [x] 9.3 Render the inventory table: id, status (cyan/amber badge), last seen, RAM (MiB), GPU count, accelerators
- [x] 9.4 Render an empty-state block when the list is empty, linking to the existing operator-invite generator
- [x] 9.5 Implement row click → side panel calling `get_node_capabilities(node_id)` with per-GPU breakdown
- [x] 9.6 Implement the debounced "Refresh" control (max one call / 1500 ms)
- [x] 9.7 Add `{ route: "nodes", label: "Nodes", tagName: "tachyon-nodes-panel" }` in `tachyon-ui/src/registry/ComponentRegistry.ts`, ordered above `fleet`
- [x] 9.8 Update i18n dictionaries (`nav.nodes`, `nodes.title`, `nodes.column.*`, `nodes.empty.*`)
- [x] 9.9 Unit tests: render with three nodes, render empty state, refresh debounce

## 10. UI — Systems view

- [x] 10.1 Create `tachyon-ui/src/components/domains/TachyonSystemsPanel.ts` mounting under `<tachyon-systems-panel>`
- [x] 10.2 Fetch `list_registered_systems` and `list_deployed_systems` on connect
- [x] 10.3 Render a unified table: slug, catalog version, status (`not-deployed` / `deployed` / `version-drift`), host-node count
- [x] 10.4 Implement row expand → per-node `(node_id, version)` entries; show the "not currently active" placeholder when `not-deployed`
- [x] 10.5 Verify no mutating controls are rendered (read-only contract)
- [x] 10.6 Add `{ route: "systems", label: "Systems", tagName: "tachyon-systems-panel" }` in `ComponentRegistry.ts`, after `nodes`
- [x] 10.7 Update i18n dictionaries
- [x] 10.8 Unit tests: 35 catalog entries with mixed states; expand interaction; read-only assertion

## 11. UI — fix existing screens

- [x] 11.1 In `TachyonOverviewPanel.ts`, replace `snapshot.batchTargets.length` with `get_cluster_hardware_summary().enrolledCount` (fallback to `list_enrolled_nodes().length`)
- [x] 11.2 Update the `t("overview.nodes.detail")` copy to reference enrolled nodes rather than batch targets
- [x] 11.3 In `TachyonTopologyPanel.ts`, remove the `DEFAULT_NODES` / `DEFAULT_EDGES` constants and the implicit fallback in `loadLiveTopology`
- [x] 11.4 Add an empty-state block rendered when `get_topology_graph` returns an empty node list, linking to the `nodes` route
- [x] 11.5 Extract the legacy sample graph into a new `topology.demo.ts` module and load it only when `window.location.search` contains `demo=1`; render the "Demo data — not connected" banner above the canvas
- [x] 11.6 Ensure the live/offline banner accurately reflects the real `get_topology_graph` source, independent of the demo flag

## 12. End-to-end verification

- [ ] 12.1 Start the Tauri dev shell against a freshly initialized host store; confirm the Nodes view shows the empty state
- [ ] 12.2 Approve one enrolment from the Operator Invite flow; confirm the new node appears with `status = "awaiting-capabilities"`
- [ ] 12.3 Bring the new node fully online and trigger a first capability heartbeat; confirm the row updates to `status = "online"` with non-zero RAM
- [ ] 12.4 In the Systems view, confirm at least the systems running on the local host appear with `status = "deployed"`
- [ ] 12.5 Reopen the Topology view with no edits; confirm the empty-state card is shown and that the legacy sample nodes are absent
- [ ] 12.6 Reload with `?demo=1`; confirm the demo banner appears and the sample graph is restored
- [ ] 12.7 Restart the host and confirm the registry is reloaded from ReDB on first read
- [x] 12.8 Update `CHANGELOG.md` with a one-line summary under the unreleased section

## 13. Documentation and archive readiness

- [x] 13.1 Update `tachyon-ui/openspec/` if any panel-level docs exist for the modified views
- [x] 13.2 Document the new MCP resources (`hardware://mesh/{node_id}/status`, cluster summary) in the MCP server's tool/resource registry
- [x] 13.3 Verify `openspec status --change "mesh-node-registry-and-systems-catalog"` shows all artifacts done and `isComplete = true`
- [x] 13.4 Run the full workspace test suite and the UI Playwright smoke tests before requesting archival
