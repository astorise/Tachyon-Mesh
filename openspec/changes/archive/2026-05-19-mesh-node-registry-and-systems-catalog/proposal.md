# Proposal: Mesh Node Registry and Systems Catalog

## Why

Tachyon-UI is currently not operable as a console. After relaunching the application, the operator cannot answer three questions the product is supposed to make trivial:

1. **Which nodes are in my fleet, and what can they do?** The "Fleet" route is a two-field policy form (`selector_tags`, `node_profile`) with no inventory. `get_hardware_status` only reads the local node (`tachyon-client::read_local_hardware_status`). There is no `list_enrolled_nodes` command anywhere in the stack.
2. **Which system-faas-* services exist, and which are running?** The repository ships ~35 `systems/system-faas-*` crates (gateway, registry, microvm-runner, k8s-scaler, dist-limiter, …). None of them are surfaced in the UI; there is no list, no enable/activate path. `system-faas-registry` is misleadingly named — it is an asset uploader for WASM blobs, not a registry of systems.
3. **What is the real state of the topology?** `TachyonTopologyPanel` falls back to a hardcoded `DEFAULT_NODES` / `DEFAULT_EDGES` (8 fake nodes) whenever `get_topology_graph` returns zero nodes, so the operator sees a fully populated graph that has nothing to do with their cluster. The Overview KPI labelled "nodes" is computed from `snapshot.batchTargets.length`, which is the number of batch routing targets, not the fleet size.

The previous UI audits stayed cosmetic and did not address this gap. This change introduces the missing backend authority (a persistent node registry + a systems catalog) and the two missing views, plus the corrections that make the Overview and Topology screens honest.

## What Changes

- Introduce a new WASM control-plane FaaS at `systems/system-faas-node-registry` (built against the `control-plane-faas` world) that owns the enrollment ceremony and the persisted set of enrolled nodes with their declared capabilities (RAM, VRAM, accelerators, region, last-seen). The operator-side `EnrollmentManager` code currently living in `core-host/src/node_enrollment.rs` migrates **into** this FaaS; `core-host` retains only the HTTP routes (`/admin/enrollment/*`, `/admin/nodes/*`) that forward to the FaaS `handle-request` export. Persistence flows through the existing `kv-partition` WIT import, backed by a dedicated ReDB table inside `core-host::CoreStore`.
- Introduce a `mesh-systems-catalog` capability that exposes (a) the static list of `system-faas-*` crates known to this build and (b) the dynamic subset currently active on the mesh, with versions and host nodes. The static side can be generated at build time from the workspace `Cargo.toml`; the dynamic side is derived from the node registry's per-node `active_systems` field.
- Extend `tachyon-client` with four new functions: `list_enrolled_nodes()`, `get_node_capabilities(node_id)`, `list_registered_systems()`, `list_deployed_systems()`. Each is exposed as a `#[tauri::command]` in `tachyon-ui/src/main.rs`.
- Add a new UI route **`nodes`** with `TachyonNodesPanel`: a fleet inventory (id, status, last-seen, capabilities columns). `TachyonFleetPanel` is preserved as the policy form but moved behind the inventory.
- Add a new UI route **`systems`** with `TachyonSystemsPanel`: catalog list with per-system status (registered/deployed), version, and host count. Read-only in this change.
- **BREAKING (behavioral)** `TachyonTopologyPanel` no longer renders `DEFAULT_NODES`/`DEFAULT_EDGES` when the backend returns an empty graph. Instead it shows an explicit empty state directing the operator to the Nodes view.
- `TachyonOverviewPanel` recomputes the "nodes" metric from `list_enrolled_nodes().length` instead of `batchTargets.length`. The "gpu" heuristic is documented as derived (a follow-up change will replace it with a real signal).
- Register the two new routes in `tachyon-ui/src/registry/ComponentRegistry.ts`.

## Capabilities

### New Capabilities
- `mesh-node-registry`: Persistent, queryable registry of enrolled mesh nodes with their declared hardware capabilities and last-seen status. Implemented as the `system-faas-node-registry` crate; consumed by host commands and by the systems catalog.
- `mesh-systems-catalog`: Authoritative view of which `system-faas-*` components this build ships and which are currently active on the mesh, with per-system version and host-node list.
- `tachyon-ui-nodes-view`: Operator-facing inventory view (`<tachyon-nodes-panel>`, route `nodes`) that lists enrolled nodes and their capabilities, sourced from the mesh node registry.
- `tachyon-ui-systems-view`: Operator-facing catalog view (`<tachyon-systems-panel>`, route `systems`) that lists registered and deployed `system-faas-*` components.

### Modified Capabilities
- `topology-canvas-taxonomy`: Add a requirement that the topology panel renders an explicit empty state when the backend returns zero nodes; remove the implicit fallback to sample data. Existing taxonomy, drag, editor, and serialize requirements are untouched.
- `hardware-capabilities`: Extend the MCP/host surface from local-only to mesh-wide. Today the spec requires `hardware://local/status`; this change adds a requirement for retrieving any enrolled node's capabilities through the mesh node registry, while keeping `local/status` as the source for the host's own data.

## Impact

- **New FaaS crate**: `systems/system-faas-node-registry` (Rust + WASM control-plane-faas guest, importing `kv-partition`). Added to the workspace `Cargo.toml` members.
- **Affected crates**:
  - `core-host` — the operator-side `EnrollmentManager` is removed from `src/node_enrollment.rs`; replaced by HTTP forwarders to the FaaS. A new ReDB table named `node-registry` is exposed through the existing `kv-partition` host implementation.
  - `tachyon-client` — four new public async functions; new serde types `EnrolledNode`, `NodeCapabilities`, `RegisteredSystem`, `DeployedSystem`.
  - `tachyon-ui` — four new `#[tauri::command]` entries; new components, new routes, modifications to `TachyonOverviewPanel` and `TachyonTopologyPanel`.
- **WIT**: One-line extension of the `control-plane-faas` world in `wit/tachyon.wit` to import `kv-partition`. No new WIT interface is introduced — persistence reuses the existing `kv-partition::table` resource (string keys, byte-blob values).
- **Schemas**: New JSON schemas under `tachyon-ui/gen/schemas/` for nodes and systems payloads.
- **Out of scope** for this change (documented to keep the diff bounded):
  - Activate/deactivate flow for `system-faas-*` components (this change is read-only for the catalog).
  - State sections on the six policy-only panels (resilience, identity-config, rbac, supply-chain, ai, workloads).
  - Replacing the Overview "gpu" heuristic with a real cluster GPU signal.
  - Any modification to the enrollment protocol itself; we only consume join events.
