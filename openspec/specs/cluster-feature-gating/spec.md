# cluster-feature-gating Specification

## Purpose
Defines how Tachyon derives cluster feature availability from compiled-in systems and surfaces those flags to the UI for navigation filtering and route guarding.

## Requirements

### Requirement: core-host MUST populate active_systems from the sealed manifest routes
The `self_registry_node()` function in core-host SHALL derive the `active_systems` list by reading the current `IntegrityConfig.routes` (via `state.runtime.load().config.routes`), filtering those with `role == RouteRole::System`, and mapping each to `RegistryActiveSystem { slug: path.strip_prefix("/system/"), version }`. It SHALL NOT hardcode an empty list.

#### Scenario: Node with system routes compiled in reports them in active_systems
- **WHEN** a node's sealed manifest includes routes `/system/gateway` and `/system/authn` with `role: system`
- **THEN** `self_registry_node()` produces `active_systems` containing entries with slugs `"gateway"` and `"authn"`

#### Scenario: Node with no system routes reports empty active_systems
- **WHEN** a node's sealed manifest contains no routes with `role: system`
- **THEN** `self_registry_node()` produces `active_systems: []`

### Requirement: Backend MUST derive feature flags from active_systems slugs only
The Tachyon backend SHALL provide a `get_cluster_features` command that calls `list_enrolled_nodes()`, collects the union of `active_systems` slugs across all enrolled nodes, and returns a `ClusterFeatureSet` struct. Each flag SHALL be derived solely from slug membership — not from hardware metrics, mounted volumes, or runtime configuration.

#### Scenario: Features returned for a cluster with AI and identity systems compiled in
- **WHEN** at least one enrolled node has `ai-list-model` in its `active_systems` and another has `authn`
- **THEN** `get_cluster_features` returns `hasAi: true` and `hasIdentity: true`

#### Scenario: Features absent when systems are not compiled in
- **WHEN** no enrolled node has `s3-proxy` or `storage-broker` in its `active_systems`
- **THEN** `get_cluster_features` returns `hasStorage: false` regardless of whether S3 volumes are mounted

#### Scenario: Features returned for an empty cluster
- **WHEN** no nodes are enrolled
- **THEN** `get_cluster_features` returns all fields as `false`

### Requirement: Frontend MUST cache cluster features in a reactive store
The frontend SHALL maintain a `clusterFeaturesStore` (Zustand vanilla) that holds the last-fetched `ClusterFeatureSet` and a `status` field (`"loading" | "ready" | "error"`). The store SHALL fetch features on every `connection:connected` event and SHALL NOT require manual refresh.

#### Scenario: Store fetches on connection established
- **WHEN** a `connection:connected` event is dispatched
- **THEN** the store transitions to `status: "loading"`, calls `get_cluster_features`, then transitions to `status: "ready"` with the result

#### Scenario: Store handles backend error gracefully
- **WHEN** `get_cluster_features` returns an error
- **THEN** the store transitions to `status: "error"` and sets `features` to `null`

#### Scenario: Consumers receive reactive updates
- **WHEN** a component subscribes to `clusterFeaturesStore` before a reconnect cycle
- **THEN** it receives the updated `ClusterFeatureSet` without requiring a page reload

### Requirement: ClusterFeatureSet MUST cover all gated navigation panels
The `ClusterFeatureSet` SHALL include at minimum: `hasEnrolledNodes`, `hasFleet`, `hasAi`, `hasRouting`, `hasResilience`, `hasIdentity`, `hasRbac`, `hasStorage`, `hasObservability`, `hasSupplyChain`. Additional fields MAY be added without a breaking change.

#### Scenario: Fleet flag requires more than one node
- **WHEN** exactly one node is enrolled
- **THEN** `hasFleet` is `false`
- **WHEN** two or more nodes are enrolled
- **THEN** `hasFleet` is `true`

#### Scenario: AI flag driven by compiled system slugs
- **WHEN** no enrolled node has `ai-list-model`, `model-broker`, or `buffer` in `active_systems`
- **THEN** `hasAi` is `false`
- **WHEN** at least one enrolled node has any of those slugs in `active_systems`
- **THEN** `hasAi` is `true`
