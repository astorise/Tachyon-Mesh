## 1. Foundation: scope data model and parser

- [x] 1.1 Add `globset` dependency to `core-host/Cargo.toml` (if not already present).
- [x] 1.2 Create `core-host/src/host_core/scoping/mod.rs` with the public surface: `DeploymentScopes`, `ScopeShape`, `ScopeCategory`, `ScopeCheck`, `LinkerCache`.
- [x] 1.3 Implement `ScopeCategory` enum covering `Secrets`, `Kv`, `Vector`, `Training`, `Bridge`, `Routing`, `Http`, `Outbox`, `Storage`, `Graph`. Each variant carries the per-category compiled patterns where applicable.
- [x] 1.4 Implement `DeploymentScopes::from_manifest(value: &Value) -> Result<Self, ScopeManifestError>` that compiles globs and enforces the `routing:` tuple rule (route-path → destination).
- [x] 1.5 Implement the `allow-all` sentinel branch in the parser; emit a `tracing::warn!` and a counter increment when matched.
- [x] 1.6 Implement `ScopeShape::of(&DeploymentScopes) -> ScopeShape` returning a normalized, hashable description of granted categories and their pattern sets (sort + dedupe before hashing).
- [x] 1.7 Implement `ScopeManifestError` with variants for unknown category, non-string pattern, routing missing destination, uncompilable glob. Each variant carries the offending value for the operator-facing message.
- [x] 1.8 Unit-test `DeploymentScopes::from_manifest` for the happy path, every error variant, the `allow-all` sentinel, and equivalence of `ScopeShape` under reordering of patterns.

## 2. Manifest schema and validation wiring

- [x] 2.1 Locate the manifest type that drives [component_hosts.rs](core-host/src/host_core/component_hosts.rs) deployment creation; add a `scopes: serde_yaml::Value` (or equivalent untyped) field that the loader passes to `DeploymentScopes::from_manifest`.
- [x] 2.2 Wire the parser into the manifest submission/validation path so malformed scopes are rejected before reaching the runtime (see spec: "Manifest validation rejects malformed scopes at submission").
- [x] 2.3 Default missing `scopes:` to `DeploymentScopes::allow_all()` with the warning + counter (see spec: "Manifest omits scopes entirely").
- [x] 2.4 Extend the manifest schema documentation in `docs/` (or whichever location currently documents deployment manifests) with the new `scopes:` block syntax and the per-category pattern syntax.
- [x] 2.5 Add integration test: submit a manifest with `scopes.secrest:` typo → submission rejected with a clear error.
- [x] 2.6 Add integration test: submit a manifest with `scopes.routing:` entry missing destination → submission rejected.

## 3. Linker cache

- [x] 3.1 Define `LinkerCache` as `DashMap<ScopeShape, Arc<Linker<StoreData>>>` backed by an LRU bound (default 256, exposed via existing host config). Choose between `lru` crate + mutex or a sharded `moka`-style cache — pick whichever matches existing `core-host` dependency style.
- [x] 3.2 Implement `LinkerCache::get_or_build(&self, shape: &ScopeShape, builder: impl Fn() -> Linker<StoreData>) -> Arc<Linker<StoreData>>` with hit/miss counters wired to the existing metrics module.
- [x] 3.3 Expose `faas_linker_cache_hit_total` and `faas_linker_cache_miss_total` counters via the host's existing prometheus exporter.
- [x] 3.4 Unit-test `LinkerCache` with: cache hit on identical shape; cache miss on different shape; LRU eviction at the bound; concurrent gets returning the same `Arc`.

## 4. Scoped Linker builder

- [x] 4.1 In [guest_runtime.rs](core-host/src/host_core/guest_runtime.rs), refactor `build_faas_linker`, `build_udp_linker`, `build_websocket_linker`, `build_system_linker`, `build_background_linker`, `build_control_plane_linker` (or whichever the current function names are) to accept a `&ScopeShape` instead of building an unconditional linker.
- [x] 4.2 Wrap each existing `tachyon::mesh::<iface>::add_to_linker::<...>` call in a `if shape.grants(ScopeCategory::<X>)` gate. Document the link-time semantic in a one-line comment per gate.
- [x] 4.3 Route every callsite that today builds a linker through `LinkerCache::get_or_build`.
- [x] 4.4 Confirm `wasmtime_wasi::p2::add_to_linker_sync` and pure host-side imports (`custom-metrics`, `telemetry-reader`, `scaling-metrics`) remain unconditional — these are infrastructure, not authorization-gated.
- [x] 4.5 Add a unit test that builds a linker for a shape lacking `bridge`, instantiates a fake `faas-guest` component importing `bridge-controller`, and asserts the instantiation error names the missing import.

## 5. StoreData and per-interface scoped closures

- [x] 5.1 Extend `StoreData` (or the equivalent per-store context in `core-host`) with a `scopes: Arc<DeploymentScopes>` field populated at guest instantiation. Keep it `Arc` so cloning across host calls is cheap.
- [x] 5.2 Rewrite the host closure for `secrets-vault.get-secret` to: read `store.data().scopes.secrets.is_match(&name)`; on false, return `secrets-vault::error::permission-denied` + denial counter increment; on true, fall through to existing logic.
- [x] 5.3 Rewrite the host closure for `kv-partition.table::new(name)` to: validate against `scopes.kv`; on false, return `Err("permission denied: kv:<name> not granted")`; on true, construct the resource as today. Document the invariant that subsequent methods on the handle do not re-check.
- [x] 5.4 Rewrite the host closures for `vector.create-index`, `upsert`, `search`, `remove` to validate the `index-name` against `scopes.vector` per call.
- [x] 5.5 Rewrite the host closure for `training.submit-training-job` to validate `job.dataset.volume-alias` against `scopes.training`.
- [x] 5.6 Rewrite the host closure for `bridge-controller.create-bridge(config)` to validate both `client-a-addr` and `client-b-addr` against `scopes.bridge` (a per-call check at construction).
- [x] 5.7 Rewrite the host closure for `routing-control.update-target(route-path, destination)` to enforce the routing tuple rule: there MUST be at least one `scopes.routing` entry whose route-path-glob matches `route-path` AND whose destination-glob matches `destination`.
- [x] 5.8 Rewrite the host closure for `outbound-http.send-request(method, url, ...)` to extract `scheme://host/path` (drop query string), and match against `scopes.http`.
- [x] 5.9 Rewrite the host closures for `outbox-store.claim-events(db-url, table, ...)` and `ack-event(db-url, table, id)` to validate `<db-url>/<table>` against `scopes.outbox`.
- [x] 5.10 Rewrite the host closures for `storage-broker.enqueue-write(path, ...)`, `snapshot-volume(volume-id, ...)`, `restore-volume(volume-id, ...)` to validate the relevant argument against `scopes.storage`.
- [x] 5.11 Rewrite the host closure for `graph.workspace-graph::new(name)` to validate against `scopes.graph` at construction; document handle-bound invariant.
- [x] 5.12 Confirm no closure captures tenant data inside the linker (only reads from `store.data().scopes`). Add a one-line comment at the top of `scoping/mod.rs` recording this invariant.

## 6. Counters, logs, and denial reporting

- [x] 6.1 Add prometheus counters: `faas_scopes_allow_all_total{deployment}`, `faas_scope_denials_total{deployment, category}`, `faas_link_denials_total{deployment, interface}`.
- [x] 6.2 Implement sampled denial WARN logging: track a per-deployment denial rate; when the rate crosses a configured threshold (default 100/min), emit one WARN log with deployment + category. Otherwise increment counters silently.
- [x] 6.3 Surface per-deployment denial counts in the existing telemetry snapshot (extend the snapshot serializer if needed; do NOT change the `metrics-snapshot` WIT record).
- [x] 6.4 Document the new metrics in the operator-facing observability docs.

## 7. Integration tests (host side)

- [x] 7.1 Add `core-host/src/host_core/tests/scoping_link_time.rs`: instantiate a `faas-guest` component importing `bridge-controller` under a deployment without the `bridge` category → assert wasmtime link error.
- [x] 7.2 Add `core-host/src/host_core/tests/scoping_secrets.rs`: instantiate under `scopes.secrets: ["db/prod/*"]`, call `get-secret("db/prod/p")` → ok; call `get-secret("auth/master")` → permission-denied + counter +1.
- [x] 7.3 Add `core-host/src/host_core/tests/scoping_kv_handle.rs`: instantiate under `scopes.kv: ["tenant-a/*"]`, open `tenant-a/users` → ok; open `tenant-b/users` → err; after `tenant-a/users` open, perform 1000 `get`/`set` calls and assert no additional denial counter increments (handle-bound).
- [x] 7.4 Add `core-host/src/host_core/tests/scoping_routing_tuple.rs`: assert that a `routing` entry with only a route-path glob is rejected at manifest parse; assert that update-target with matching route-path but non-matching destination is denied.
- [x] 7.5 Add `core-host/src/host_core/tests/scoping_outbound_http.rs`: query string ignored for matching; non-matching host denied.
- [x] 7.6 Add `core-host/src/host_core/tests/scoping_linker_cache.rs`: two deployments with semantically equal scope blocks share a `Linker`; LRU eviction returns to cache miss; concurrent gets return the same `Arc`.
- [x] 7.7 Add `core-host/src/host_core/tests/scoping_disjoint_capabilities.rs`: assert that adding/removing scope categories does NOT alter the deployment's `capability_mask` or routing selection.

## 8. Migration default and ratchet

- [x] 8.1 Confirm every existing FaaS manifest in `systems/system-faas-*/` and `examples/guest-*/` continues to instantiate under default `allow-all`. Add a regression test that loads each in turn.
- [x] 8.2 Add a node-level config flag `require_scopes: bool` (default `false`); when `true`, manifests resolving to `allow-all` MUST be rejected at submission.
- [x] 8.3 Document the four-phase migration plan from `design.md` in `docs/` so operators can plan their tightening rollout.
- [x] 8.4 Do NOT flip the default in this change. A separate openspec change will own that flip when telemetry shows zero `allow-all` deployments.

## 9. Documentation

- [x] 9.1 Add a new section "Import scoping" to the FaaS operator guide explaining `scopes:` syntax, per-category patterns, and the link-time vs. runtime distinction.
- [x] 9.2 Add a one-paragraph note at the top of `wit/tachyon.wit` (as a comment) stating that this file is the single source of truth and that per-deployment authorization is layered on top by the host (link in `feedback_wit_world_single_source` rationale).
- [x] 9.3 Document the resource-bound invariant near each resource constructor in `wit/tachyon.wit` (comment only; no signature change): "host validates this constructor argument against deployment scopes; subsequent methods do not re-check".

## 10. Pre-merge validation

- [x] 10.1 Workspace test run green with every system FaaS rebuilt against the existing world (no rebuild expected; assert this).
- [ ] 10.2 `cargo bench` (or equivalent micro-bench) confirms per-call overhead for value-based imports stays under 100 ns p99.
- [ ] 10.3 End-to-end smoke: deploy a multi-tenant `faas-guest` with `scopes.kv: ["tenant-a/*"]` and another with `scopes.kv: ["tenant-b/*"]`; confirm cross-tenant read attempts are denied.
- [x] 10.4 Confirm `wit/tachyon.wit` has zero non-whitespace diff vs. main before merge. (Only documentation comments added per task 9.3; no signature, world, or record changes.)
