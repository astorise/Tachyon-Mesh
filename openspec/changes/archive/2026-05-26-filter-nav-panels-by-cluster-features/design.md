## Context

The Tachyon UI navigation is built from a static `ComponentRegistry.ts` that unconditionally lists 16 panels. The `TachyonAppShellNav` component renders all of them on every connect, even when large portions of the feature surface are absent from the cluster.

Each node enrolled in the cluster reports which system components are compiled into its binary via the `active_systems` field of `NodeCapabilities` (a `Vec<ActiveSystem>` where each entry carries a `slug` and `version`). These slugs are the canonical identifiers defined in `systems/manifest.toml` (e.g., `authn`, `gateway`, `ai-list-model`, `s3-proxy`). A feature is considered available if at least one enrolled node has the corresponding system slug in its `active_systems`.

What is missing is a single query that maps the union of `active_systems` across all nodes to a discrete set of UI feature flags, and a store + registry integration to consume it.

## Goals / Non-Goals

**Goals:**
- Single backend command (`get_cluster_features`) that returns a flat `ClusterFeatureSet` with boolean flags.
- Frontend store that fetches features on connect and on reconnect, with no manual refresh needed.
- Registry-level `requires` field so feature-to-panel mapping is co-located with the panel definition.
- Nav silently hides unavailable panels; no error state shown to the user.
- Redirect to overview when navigating directly (via URL hash) to an unavailable panel.

**Non-Goals:**
- Granular permission gating (that is RBAC, not feature availability).
- Showing panels in a "disabled" visual state — absent is cleaner than greyed-out for missing features.
- Server-push updates to feature flags while the shell is open (polling on reconnect is sufficient).
- Changing the Tauri command surface for any panel's own data fetching.

## Decisions

### D1 — Single aggregated backend command, not per-panel probes

**Decision:** Add one `get_cluster_features() → ClusterFeatureSet` Tauri command that the frontend calls once on connect.

**Rationale:** Calling 4–6 separate commands in parallel to gate each panel group would increase startup latency and couple the nav render to multiple independent failure modes. A single aggregated call is atomic from the UI's perspective and trivially cacheable.

**Alternative considered:** Per-panel lazy probes (each panel self-reports availability when first mounted). Rejected because the nav would have to mount hidden panels just to check, and the user would see the nav settle/shift after load.

### D2 — Feature flags as a flat set of named booleans

**Decision:** `ClusterFeatureSet` is a struct of `bool` fields (e.g., `has_enrolled_nodes`, `has_gpu`, `has_identity_domain`), serialised to camelCase JSON.

**Rationale:** A string enum set would require backend→frontend synchronisation of string literals. Booleans with well-named fields are self-documenting, trivially serialisable via `serde`, and safe to extend additively without a breaking change.

**Alternative considered:** A `Vec<String>` feature list (like a capability set). Rejected because pattern-matching on strings in TypeScript is error-prone and harder to type-check.

### D3 — `requires` on `ComponentRoute` is a single `ClusterFeature` key

**Decision:** Each route declares at most one `requires?: ClusterFeature` field. `ClusterFeature` is a TypeScript union of the camelCase field names of `ClusterFeatureSet`.

**Rationale:** No panel currently needs two independent conditions. If a future panel does, the `requires` field can be widened to an array without breaking existing routes.

### D4 — Nav reads features from a Zustand store, re-renders reactively

**Decision:** `clusterFeaturesStore` (Zustand vanilla) holds `{ features: ClusterFeatureSet | null, status: "loading" | "ready" | "error" }`. `TachyonAppShellNav` subscribes to the store and re-renders when it changes.

**Rationale:** Consistent with the existing `connectionStore` pattern. The store decouples data-fetching from rendering and makes it testable in isolation.

**Alternative considered:** Fetching inside `TachyonAppShellNav.connectedCallback()`. Rejected because the nav is re-mounted on reconnect; centralising the fetch avoids duplicate calls.

### D5 — active_systems must be wired from the sealed manifest before use

**Decision:** Add a step in `self_registry_node()` in `core-host/src/host_core/integrity_config.rs` that populates `active_systems` by reading `state.runtime.load().config.routes`, filtering on `role == RouteRole::System`, and mapping each to `RegistryActiveSystem { slug: path.trim_start_matches("/system/"), version }`.

**Rationale:** Currently `active_systems` is hardcoded to `Vec::new()`. The sealed manifest (`IntegrityConfig.routes`) is the authoritative list of system-faas components compiled into the binary — it is embedded at build time and verified by the integrity lock. Deriving `active_systems` from it makes UI feature visibility reflect actual binary composition, not runtime state.

**Alternative considered:** Deriving flags from `get_mesh_graph()` live routes — rejected because those reflect currently-running routes, which can differ from compiled-in routes (e.g., a system route could fail to start without recompiling the binary).

### D6 — Feature flag computation aggregates active_systems across all nodes

**Decision:** `get_cluster_features()` in `tachyon-client` calls `list_enrolled_nodes()`, collects the union of all `active_systems` slugs across enrolled nodes, then derives boolean flags by checking slug membership. No other API calls.

**Rationale:** Once `active_systems` is correctly wired (D5), it becomes the single source of truth. Aggregating across all nodes means the UI shows a panel if any node in the cluster has the capability — which matches operator expectations for a distributed mesh.

## Feature flag mapping

`ClusterFeatureSet` is computed from `list_enrolled_nodes()` only. All flags except `hasEnrolledNodes` and `hasFleet` are derived by checking whether the union of `active_systems` slugs across all enrolled nodes contains at least one of the listed slugs (from `systems/manifest.toml`).

| `ClusterFeatureSet` field | Condition |
|---|---|
| `hasEnrolledNodes` | at least one enrolled node |
| `hasFleet` | `enrolled_count > 1` |
| `hasAi` | any node has `ai-list-model`, `model-broker`, or `buffer` in `active_systems` |
| `hasRouting` | any node has `gateway` or `mesh-overlay` in `active_systems` |
| `hasResilience` | any node has `shadow-proxy` or `dist-limiter` in `active_systems` |
| `hasIdentity` | any node has `authn` in `active_systems` |
| `hasRbac` | any node has `authz` in `active_systems` |
| `hasStorage` | any node has `s3-proxy` or `storage-broker` in `active_systems` |
| `hasObservability` | any node has `otel`, `prom`, or `logger` in `active_systems` |
| `hasSupplyChain` | any node has `registry` or `gitops-broker` in `active_systems` |

| Panel route | `requires` |
|---|---|
| `overview` | — (always shown) |
| `topology` | `hasEnrolledNodes` |
| `nodes` | `hasEnrolledNodes` |
| `hardware` | `hasEnrolledNodes` |
| `systems` | `hasEnrolledNodes` |
| `workloads` | `hasEnrolledNodes` |
| `fleet` | `hasFleet` |
| `ai` | `hasAi` |
| `routing` | `hasRouting` |
| `resilience` | `hasResilience` |
| `users` | `hasIdentity` |
| `identity-config` | `hasIdentity` |
| `rbac` | `hasRbac` |
| `storage` | `hasStorage` |
| `observability` | `hasObservability` |
| `supply-chain` | `hasSupplyChain` |

## Risks / Trade-offs

- **Stale features on long sessions** → The store re-fetches on every `connection:connected` event; operators who keep the UI open overnight will see updates on next reconnect. Acceptable for v1.
- **`get_cluster_features` fails** → Store falls back to `null`; nav shows only always-visible panels (overview). Better than showing broken panels.
- **Slug set changes between manifest versions** → If a slug is renamed in `manifest.toml`, the Rust flag derivation must be updated in sync. This is a single-file change and caught at compile time if the mapping is expressed as constants.
- **`observability` hidden when no obs system compiled** → This is intentional: if no `otel`/`prom`/`logger` system is compiled in, the Observability panel has nothing to display. Operators can always reach the Overview for basic status.
