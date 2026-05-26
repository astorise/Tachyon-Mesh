## 1. core-host — Wire active_systems from the sealed manifest

- [x] 1.1 In `core-host/src/host_core/integrity_config.rs`, update `self_registry_node()` to load the current runtime config via `state.runtime.load()`, filter `config.routes` by `role == RouteRole::System`, and map each to `RegistryActiveSystem { slug: route.path.trim_start_matches("/system/").to_owned(), version: route.version.clone() }` — replace the hardcoded `active_systems: Vec::new()` with this derived list
- [x] 1.2 Verify the Rust build compiles without warnings (`cargo build -p core-host`)

## 2. Rust — ClusterFeatureSet and backend command

- [x] 2.1 Add `ClusterFeatureSet` struct to `tachyon-client/src/lib.rs` with boolean fields: `has_enrolled_nodes`, `has_fleet`, `has_ai`, `has_routing`, `has_resilience`, `has_identity`, `has_rbac`, `has_storage`, `has_observability`, `has_supply_chain` (derive `Serialize`, `Default`, serde camelCase rename)
- [x] 2.2 Add `pub async fn get_cluster_features() -> Result<ClusterFeatureSet>` in `tachyon-client/src/lib.rs` that: (a) calls `list_enrolled_nodes()`, (b) collects the union of all `node.capabilities.active_systems` slugs into a `HashSet<&str>`, (c) sets `has_enrolled_nodes = !nodes.is_empty()`, `has_fleet = nodes.len() > 1`, and each other flag by checking if any of the relevant slugs is in the set (see design.md mapping table)
- [x] 2.3 Add `#[tauri::command] async fn get_cluster_features()` in `tachyon-ui/src/main.rs` delegating to the client fn
- [x] 2.4 Register `get_cluster_features` in the Tauri `invoke_handler` builder in `main.rs`
- [x] 2.5 Verify the Rust build compiles without warnings (`cargo build -p tachyon-ui`)

## 3. TypeScript — ClusterFeature type and ComponentRegistry update

- [x] 3.1 Export a `ClusterFeatureSet` TypeScript interface in a new file `tachyon-ui/src/types/clusterFeatures.ts` mirroring the Rust struct: `hasEnrolledNodes`, `hasFleet`, `hasAi`, `hasRouting`, `hasResilience`, `hasIdentity`, `hasRbac`, `hasStorage`, `hasObservability`, `hasSupplyChain` (all `boolean`)
- [x] 3.2 Add `ClusterFeature` union type (`keyof ClusterFeatureSet`) to `tachyon-ui/src/registry/ComponentRegistry.ts`
- [x] 3.3 Add optional `requires?: ClusterFeature` field to the `ComponentRoute` type
- [x] 3.4 Annotate each route entry in the `routes` array with its `requires` value per the mapping table in `design.md` (`overview` has no `requires`)

## 4. TypeScript — clusterFeaturesStore

- [x] 4.1 Create `tachyon-ui/src/stores/clusterFeaturesStore.ts` with a Zustand vanilla store holding `{ features: ClusterFeatureSet | null, status: "loading" | "ready" | "error" }`
- [x] 4.2 Implement `fetchFeatures()` action in the store that calls `invoke("get_cluster_features")` via `resilientInvoke`, sets `status: "loading"` before the call, `status: "ready"` on success, and `status: "error"` on failure
- [x] 4.3 Subscribe to `connection:connected` window event in the store module init to call `fetchFeatures()` automatically on each (re)connect
- [x] 4.4 Export a `isFeatureAvailable(feature: ClusterFeature): boolean` helper that reads `clusterFeaturesStore.getState()` and returns `true` when `features` is non-null and the named field is `true`

## 5. TypeScript — TachyonAppShellNav filtering

- [x] 5.1 Import `clusterFeaturesStore` and `isFeatureAvailable` in `TachyonAppShellNav.ts`
- [x] 5.2 Subscribe to `clusterFeaturesStore` in `connectedCallback` and call `this.render()` on each store change; unsubscribe in `disconnectedCallback`
- [x] 5.3 In `TachyonAppShellNav.render()`, filter `listComponentRoutes()` to exclude routes whose `requires` field is set but `isFeatureAvailable(route.requires)` returns `false`
- [x] 5.4 Verify that routes without a `requires` field (`overview`) are always rendered regardless of store state

## 6. TypeScript — TachyonAppShell route guard

- [x] 6.1 In `TachyonAppShell.ts`, in the `showRoute()` method (or equivalent routing logic), after resolving the route, check if the target `ComponentRoute` has a `requires` field and `isFeatureAvailable` returns `false`
- [x] 6.2 If the route is unavailable, redirect to `overview` by setting `window.location.hash = "overview"` and calling `showRoute("overview")`
- [x] 6.3 Ensure the redirect does not loop (`overview` has no `requires`, so it is always available)

## 7. Verification

- [x] 7.1 Run `npm run typecheck` (or equivalent) in `tachyon-ui/` and confirm no new type errors
- [x] 7.2 Run `npm run build` and confirm the bundle builds cleanly
- [x] 7.3 Start the app against a cluster with only base systems compiled in and confirm feature-gated panels are absent from the nav
- [x] 7.4 Navigate directly to a gated route (e.g., `#ai`) on a cluster without the AI system and confirm redirect to `#overview`
- [x] 7.5 Confirm `overview` is always visible regardless of cluster state
