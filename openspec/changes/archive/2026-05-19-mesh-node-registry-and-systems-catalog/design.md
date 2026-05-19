# Design: Mesh Node Registry and Systems Catalog

## Context

Tachyon's enrollment flow today (`core-host/src/node_enrollment.rs` + the `admin_enrollment_poll_handler` in `host_core/integrity_config.rs`) signs a CSR and hands a certificate back over a long-poll. The signed identity then flies off to the new node, and **nothing in the host persists the fact that a node now exists**. As a consequence:

- `tachyon-client` exposes no way to ask "which nodes are in the mesh?". The closest neighbour, `MeshGraphSnapshot`, returns routing routes and batch targets — not a fleet.
- `get_hardware_status` (`tachyon-ui/src/main.rs:529-531`) calls `read_local_hardware_status()`, which scans the host process's own RAM/GPU. Cluster-wide hardware visibility does not exist.
- The 35 `systems/system-faas-*` crates are referenced from the workspace `Cargo.toml` and from manifests, but the UI has no entry point to inspect them. `system-faas-registry` is an asset uploader (it persists WASM blobs to disk, see `systems/system-faas-registry/src/lib.rs:23-62`), not a registry of which systems exist.
- `TachyonTopologyPanel` ships `DEFAULT_NODES`/`DEFAULT_EDGES` (`tachyon-ui/src/components/domains/TachyonTopologyPanel.ts:731-750`) and uses them as a fallback whenever `get_topology_graph()` returns zero nodes. The fallback is indistinguishable from a real graph to an operator who doesn't read the code.
- `TachyonOverviewPanel.metricsFromSources` (`tachyon-ui/src/components/domains/TachyonOverviewPanel.ts:132-142`) labels `batchTargets.length` as the "nodes" KPI. It is the number of batch routing targets, not the fleet.

Architectural context: the workspace already follows a convention where mesh-wide stateful subsystems are packaged as `systems/system-faas-*` crates with a WIT interface, and the host (`core-host`) plumbs them via `wit_bindgen`. That convention is the right home for a node registry.

## Goals / Non-Goals

**Goals:**
- Establish a single source of truth for enrolled nodes that survives host restarts and answers `list_enrolled_nodes()` and `get_node_capabilities(node_id)` in O(1) for small fleets (≤ 10⁴ nodes).
- Establish a catalog that distinguishes (a) the `system-faas-*` crates this build knows about from (b) those actually active on the mesh, with the host nodes that run each.
- Surface both through `tachyon-client` and `#[tauri::command]` so two new UI views can be implemented without further backend work.
- Make the Topology and Overview views honest: no fake nodes, no misleading KPI.
- Keep the diff bounded; do not redesign the enrollment protocol nor add system-install workflows.

**Non-Goals:**
- Installing or removing `system-faas-*` components at runtime. Read-only catalog only.
- Replacing the Overview "gpu" heuristic with a real cluster GPU signal. That belongs to a separate change once the registry exposes GPU stats per node.
- Implementing eventual consistency across multiple peer hosts. The first cut runs on a single control-plane node; replication via the gossip layer is deferred.
- Touching the six policy-only panels (resilience, identity-config, rbac, supply-chain, ai, workloads).

## Decisions

### D1. Registry is a WASM control-plane FaaS at `systems/system-faas-node-registry`

**Decision.** Create `systems/system-faas-node-registry` as a `control-plane-faas` WIT guest (the world defined at `wit/tachyon.wit:313-321`). The component imports `kv-partition` so it can persist rows directly, exports `handler` so the host can route HTTP traffic into it, and exports `on-tick` for the stale-status sweep. No fresh world is needed — we extend the existing `control-plane-faas` world with a `kv-partition` import (it already imports `telemetry-reader`, `outbound-http`, `routing-control`).

**Why.** The user picked this option explicitly when offered three alternatives. The convention in this repo is that mesh-state lives in `system-faas-*` WASM components, not in `core-host` Rust modules. The `control-plane-faas` world is the one designed for components that own a slice of cluster-wide state and respond to admin HTTP requests — exactly the registry's shape.

**Alternatives considered.** (a) Plain library crate hooked from `core-host` — rejected (gives the registry special host-side status that no other system has). (b) Extend `system-faas-config-api` — rejected (conflates "what is the config" with "who is in the mesh").

### D2. Operator-side enrollment logic migrates *into* the FaaS

**Decision.** The `EnrollmentManager` currently living in `core-host/src/node_enrollment.rs` is migrated into `system-faas-node-registry`. The PIN ceremony, CSR signing, and approval state machine become FaaS-internal logic. `core-host` retains only the HTTP routes (`/admin/enrollment/start`, `/admin/enrollment/approve/{session_id}`, `/admin/enrollment/poll/{session_id}`, `/admin/nodes/{node_id}/capabilities`) and forwards them to the FaaS's `handle-request` export.

**Why.** Enrollment approval is mesh state, not host state. Keeping it in `core-host` would split the registry's surface across two languages and two storage backends, and would mean every future enrollment-related change touches Rust binaries instead of a swappable WASM component. Moving it in lets the registry own one coherent surface: "node identity + capabilities + enrollment, all in one component, all behind one `kv-partition` table."

**Out of scope of this migration.** The unenrolled-node side (the outbound long-poll that ships the CSR from a fresh node) is unaffected — it does not run inside the host's FaaS runtime. Only the operator-side approval flow moves.

**Trade-off.** The node's hardware capabilities are not present at approval time (the CSR carries only the public key). We resolve this by having the node POST its `NodeCapabilities` to `/admin/nodes/{node_id}/capabilities` on first heartbeat. Until that POST arrives, the registry returns `status: "awaiting-capabilities"`.

### D3. Persistence uses ReDB through the existing `kv-partition` WIT import

**Decision.** The FaaS persists every row via `kv-partition::table` with table name `"node-registry"`. The host's existing `kv-partition` implementation maps that name to a ReDB table inside `CoreStore`. Values are `EnrolledNode` records serialized as JSON; keys are `node_id`.

**Why.** ReDB is already the single source of truth for every persistent piece of state in this repo (cwasm cache, auth cache, secrets, outbox tables — see `core-host/src/auth.rs`, `core-host/src/host_core/graph_store.rs`). The `kv-partition` WIT interface (`wit/tachyon.wit:254-268`) was designed for exactly this case: a WASM guest needing typed get/set/delete/batch/range over a named host-side table. Going around it would break the FaaS sandbox model.

**Why not a new WIT interface?** A dedicated `node-registry` WIT would be more typed (records instead of `list<u8>` blobs) but would also be a one-off — every other system uses `kv-partition`. Consistency beats type-safety for a v1 read-heavy registry; we can add a typed wrapper later if measurement shows JSON parsing is hot.

### D4. Extend the existing `control-plane-faas` world with `kv-partition`, do not fork it

**Decision.** Add `import kv-partition` directly to the existing `control-plane-faas` world in `wit/tachyon.wit`. Every current `control-plane-faas` guest is rebuilt in the same change. We do NOT introduce a parallel `control-plane-faas-with-kv` world variant.

**Why.** Forking the world would let us migrate guests one by one, but the FaaS runtime enforces a runtime cap on the number of concurrently loaded guests. If two parallel worlds existed, that cap could be hit before every guest finished migrating, and the migration would stall in a state where some guests are on the old world and unable to use the new persistence path. A single world avoids that trap entirely: every `control-plane-faas` guest has the same import surface, and the cap is never split across two ABIs.

**Trade-off.** This commits us to rebuilding every existing `control-plane-faas` guest in the same change. We accept it because the import is purely additive at the WIT level (no existing function signature changes) and the rebuild is mechanical.

**Alternatives considered.** (a) Introduce `control-plane-faas-with-kv` and migrate guests opportunistically — rejected, see "Why" above. (b) Keep the world unchanged and reach `kv-partition` through an out-of-band host call — rejected (breaks the sandbox model and contradicts D3).

### D5. `NodeCapabilities` carries `region` and `zone` as optional strings

**Decision.** `NodeCapabilities` includes `region: Option<String>` and `zone: Option<String>`, both reported by the node alongside RAM / GPU / accelerators on the capability heartbeat. Both default to `None` and are surfaced verbatim by `list_enrolled_nodes()` and the cluster summary.

**Why.** Geo-aware features (the `2026-05-17-dynamic-geo-pinning` change, future scheduling improvements) need a place to read the node's declared geography from. Adding it now costs nothing: the values flow through the existing JSON blob persisted in the `node-registry` ReDB table via `kv-partition`, so neither `core-host` nor the runtime gain any new code path. The two-level `region` / `zone` shape mirrors the convention used by every cloud provider, so node operators don't have to learn a Tachyon-specific taxonomy.

**Why not a single `location` string?** Two-level keys let downstream features filter by `region` for coarse policy and by `zone` for HA placement without re-parsing the string. The cost is one extra optional field.

### D6. `systems/manifest.toml` lives next to the systems it describes

**Decision.** Place `systems/manifest.toml` at the repository's `systems/` root, alongside the `system-faas-*` crates it lists. It is read only at build time by the `build.rs` of `system-faas-node-registry`.

**Why.** The manifest ships with the code it describes; a renamed or removed crate breaks the build in the same commit, which is the point of the cross-check assertion. Placing it under `openspec/` would conflate spec (the design contract) with artifact (the build-time data), and would force every workspace change touching a system to also reach into the OpenSpec tree.

**Core-host impact.** Zero. The manifest is consumed by `build.rs` and folded into the FaaS binary at compile time; `core-host` neither reads nor opens it at runtime.

### D4. The systems catalog has a build-time half and a runtime half

**Decision.** Generate `static_catalog.rs` at build time from a `systems/manifest.toml` (a small file listing each `system-faas-*` crate with a stable name, kebab-case slug, version, and short description). The dynamic half is derived by aggregating each node's `active_systems` field as the registry sees them.

**Why.** The static list answers "what does this Tachyon build ship?" — a build-time invariant. The dynamic list answers "what is running where?" — a runtime fact. Conflating them in one source would either be (a) wrong when a node is offline, or (b) hard to populate before any node reports in.

**Alternatives considered.** Scrape the workspace `Cargo.toml` at runtime — rejected (slow, fragile, requires shipping cargo metadata to the host). Hardcode the list in Rust — rejected (drifts from reality the moment a system is renamed).

### D5. UI: `TachyonFleetPanel` is preserved; a new `TachyonNodesPanel` is added

**Decision.** Add the new `tachyon-nodes-panel` route as the primary inventory. Keep `tachyon-fleet-panel` as the policy form, but rename its `nav.fleet` label to "Fleet Policy" and move it under the `nodes` route in the registry order.

**Why.** Deleting `TachyonFleetPanel` would also delete the policy-apply path (`applyAndSeal("fleet", …)`). The two responsibilities are distinct — inventory vs policy. Mixing them in one panel would bloat it. We add, we don't remove.

### D6. UI: `TachyonTopologyPanel` empty state, not silent fallback

**Decision.** Remove `DEFAULT_NODES`/`DEFAULT_EDGES` from `TachyonTopologyPanel.ts`. When `get_topology_graph()` returns an empty graph, render an empty-state card with a CTA pointing at the new Nodes view.

**Why.** The current fallback is a worse-than-mock: it looks real. An empty state with a clear next action is honest and faster to fix when wrong.

**Trade-off.** Developer experience: developers running the UI without a live backend will now see an empty canvas instead of a populated demo. We mitigate by exposing a `?demo=1` query flag that re-injects the legacy sample graph from a separate `topology.demo.ts` helper, used only by the dev server and Playwright tests.

## Risks / Trade-offs

- **Registry write storm during reconnects** → Mitigation: debounce capability POSTs at the FaaS boundary (single in-flight update per node, coalesce within a 2-second window) before any `kv-partition::table::set` call.
- **Stale `last_seen` after host restart** → Mitigation: the FaaS's `on-tick` sweep marks every entry as `status: "unknown"` on first tick after a host boot if no heartbeat has been seen, and lets the heartbeat loop refresh within ~30 s.
- **Static catalog drift if a system is added without updating `systems/manifest.toml`** → Mitigation: a `build.rs` assertion in `system-faas-node-registry` cross-checks the manifest entries against the workspace members at compile time and fails the build if they diverge.
- **JSON-in-`kv-partition` parsing cost grows with fleet size** → Mitigation: the registry caches the parsed list in FaaS memory across requests within the same lifetime; `list-enrolled-nodes` only re-reads the table when an `update-capabilities` or `record-approval` invalidates the cache.
- **Migrating `EnrollmentManager` mid-flight loses pending sessions** → Mitigation: pending in-memory sessions on the old host live for 15 minutes (existing TTL); we ship the FaaS migration in two commits so the old manager keeps draining its in-flight PINs while the new one starts accepting fresh ones, then we cut over once the host shows zero in-flight sessions for one TTL window.
- **`control-plane-faas` world ABI breakage** → Mitigation: per D4, every existing `control-plane-faas` guest is rebuilt in this change. The import is additive at the WIT level (no signature change to existing imports/exports), so the rebuild is mechanical. A single-world strategy is mandatory because the runtime cap on concurrently loaded guests is shared — splitting it across an old world and a `-with-kv` variant would let an opportunistic migration stall once the cap is hit.

## Migration Plan

1. Extend the `control-plane-faas` world in `wit/tachyon.wit` to import `kv-partition` (one-line change; downstream control-plane systems already in the tree are inspected for compatibility).
2. Scaffold `systems/system-faas-node-registry/` as a `control-plane-faas` guest. At this stage the crate compiles, owns a `"node-registry"` `kv-partition` table, and exposes a no-op `handle-request`. Unit tests cover the in-FaaS persistence path through a host-side `kv-partition` mock.
3. Migrate `EnrollmentManager` from `core-host/src/node_enrollment.rs` into the FaaS. Replace the host-side state with a thin HTTP forwarder: `/admin/enrollment/*` and `/admin/nodes/*` routes now call the FaaS `handle-request`. Both the old and new code paths live in parallel for one commit so tests can pivot.
4. Delete the in-host `EnrollmentManager` once the FaaS is the unique owner.
5. Implement `record-approval`, `update-capabilities`, `list-enrolled-nodes`, `get-node-capabilities`, plus the stale-status sweep in `on-tick`. All persistence goes through `kv-partition`.
6. Add the catalog logic (static from `systems/manifest.toml`, dynamic from registry rows) in the same FaaS.
7. Extend `tachyon-client` with the four new async functions and types.
8. Add the four `#[tauri::command]` entries in `tachyon-ui/src/main.rs` and regenerate any UI schemas.
9. Implement `TachyonNodesPanel` and `TachyonSystemsPanel`. Register the new routes.
10. Patch `TachyonOverviewPanel` ("nodes" KPI sourced from `list_enrolled_nodes`) and `TachyonTopologyPanel` (remove `DEFAULT_NODES` fallback + add empty state) in the same commit.
11. Smoke-test in the Tauri dev shell: enrol a node, confirm it appears in the Nodes view; stop a `system-faas-*`, confirm it leaves the active set in the Systems view.

**Rollback.** Steps 7–11 are independently revertable. The FaaS crate can stay landed (no host routes wired) without affecting the rest of the system. Step 4 (deleting the old `EnrollmentManager`) is the only one-way door — it is gated behind the parallel-run validation at the end of step 3.

## Open Questions

None outstanding. All design decisions are resolved by D1-D6 above.
