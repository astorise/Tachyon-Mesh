# faas-import-scoping Specification

## Purpose
TBD - created by archiving change faas-wit-import-scoping. Update Purpose after archive.
## Requirements
### Requirement: Deployment manifest declares import scopes

The deployment manifest of every FaaS guest SHALL carry a `scopes` block that, for each scope category, lists the argument patterns the deployment is authorized to use. A category that is absent from the block SHALL be treated as "interface denied — do not link". A category that is present with an empty list SHALL be treated as "interface linked but every call denied at runtime". A scopes value of `allow-all` SHALL be accepted as a migration default and SHALL cause a warning to be logged and a counter to be incremented for that deployment.

#### Scenario: Manifest omits scopes entirely
- **WHEN** a deployment manifest does not include a `scopes:` block
- **THEN** the host MUST resolve scopes to `allow-all`
- **AND** the host MUST log a warning naming the deployment
- **AND** the host MUST increment the `faas_scopes_allow_all_total` counter for that deployment

#### Scenario: Manifest grants a scope category
- **WHEN** the manifest contains `scopes.kv: ["tenant-a/*"]`
- **THEN** the host MUST compile the patterns into a `GlobSet` once and store it on the deployment record

#### Scenario: Manifest omits a category
- **WHEN** the manifest contains `scopes.kv: [...]` but does NOT contain `scopes.bridge`
- **THEN** the host MUST NOT register `bridge-controller.add_to_linker` in the linker for this deployment

#### Scenario: Manifest contains an empty category list
- **WHEN** the manifest contains `scopes.bridge: []`
- **THEN** the host MUST register `bridge-controller.add_to_linker` in the linker for this deployment
- **AND** every call by the guest into `bridge-controller.create-bridge` MUST be rejected at runtime with a permission-denied error

#### Scenario: Manifest contains an unknown category key
- **WHEN** the manifest contains a key that does not map to any known scope category (e.g., typo `secrest:`)
- **THEN** the manifest validation MUST reject the manifest before it reaches the linker
- **AND** the validation error MUST name the unknown key

### Requirement: Unauthorized interface imports fail at instantiation

When a guest's target WIT world imports an interface that the deployment's scopes do not grant, the host SHALL omit the corresponding `tachyon::mesh::<interface>::add_to_linker` registration from the `Linker` used to instantiate the guest. Instantiation SHALL fail with a wasmtime link error naming the missing import.

#### Scenario: Guest imports an interface its scopes do not grant
- **WHEN** a `faas-guest` component is instantiated against a deployment whose scopes do not include the `bridge` category
- **AND** the guest's WIT world imports `bridge-controller`
- **THEN** instantiation MUST fail with a wasmtime link error
- **AND** the error MUST name `tachyon:mesh/bridge-controller` as the missing import
- **AND** the failure MUST surface through the existing supervisor failure path

#### Scenario: Guest does not import the denied interface
- **WHEN** a `faas-guest` component does not actually import `routing-control`
- **AND** the deployment's scopes omit the `routing` category
- **THEN** instantiation MUST succeed (a denied interface that the guest doesn't use has no effect)

### Requirement: Resource-construction imports validate at constructor time

For WIT imports that construct a wasmtime resource keyed on an identifier — at minimum `kv-partition.table::new(name)`, `bridge-controller.create-bridge(config)`, `graph.workspace-graph::new(name)`, `vector.create-index(spec)` — the host SHALL validate the identifier(s) against the deployment's scope patterns at constructor entry. Methods on the resulting resource handle SHALL NOT re-validate the originally validated identifier.

#### Scenario: kv-partition table opened with an authorized name
- **WHEN** the deployment's `scopes.kv` is `["tenant-a/*"]`
- **AND** the guest calls `kv-partition.table.new("tenant-a/users")`
- **THEN** the constructor MUST succeed
- **AND** subsequent calls to `get`, `set`, `delete`, `batch-set`, `get-range` on the returned handle MUST NOT perform any additional scope check on the table name

#### Scenario: kv-partition table opened with an unauthorized name
- **WHEN** the deployment's `scopes.kv` is `["tenant-a/*"]`
- **AND** the guest calls `kv-partition.table.new("tenant-b/users")`
- **THEN** the constructor MUST return a permission-denied error
- **AND** no wasmtime resource handle MUST be created
- **AND** the host MUST increment a per-deployment denial counter

#### Scenario: bridge-controller create-bridge with an unauthorized address
- **WHEN** the deployment's `scopes.bridge` is `["10.0.0.0/8"]`
- **AND** the guest calls `bridge-controller.create-bridge` with `client-a-addr` outside the `10.0.0.0/8` range
- **THEN** the call MUST return a permission-denied error and no bridge MUST be created

### Requirement: Value-based imports validate per call against compiled patterns

For WIT imports that take an identifier as a function argument and do not produce a long-lived handle — at minimum `secrets-vault.get-secret(name)`, `routing-control.update-target(route-path, destination)`, `outbound-http.send-request(method, url, ...)`, `outbox-store.claim-events(db-url, table, ...)`, `outbox-store.ack-event(db-url, table, id)`, `storage-broker.enqueue-write(path, ...)`, `storage-broker.snapshot-volume(volume-id, ...)`, `storage-broker.restore-volume(volume-id, ...)`, `vector.upsert(name, ...)`, `vector.search(name, ...)`, `vector.remove(name, ...)`, `training.submit-training-job(job)` — the host SHALL validate the relevant string argument(s) against the deployment's compiled `GlobSet` for the scope category before performing any side effect. The check SHALL use the closure's reference to `StoreData` to read the compiled patterns; it SHALL NOT perform a global lookup.

#### Scenario: secrets-vault get-secret with an authorized name
- **WHEN** the deployment's `scopes.secrets` is `["db/prod/*"]`
- **AND** the guest calls `secrets-vault.get-secret("db/prod/password")`
- **THEN** the host MUST return the secret value as today

#### Scenario: secrets-vault get-secret with an unauthorized name
- **WHEN** the deployment's `scopes.secrets` is `["db/prod/*"]`
- **AND** the guest calls `secrets-vault.get-secret("auth/master")`
- **THEN** the host MUST return `secrets-vault.error::permission-denied`
- **AND** the host MUST NOT read the secret value

#### Scenario: routing-control update-target requires both route-path and destination to match
- **WHEN** the deployment's `scopes.routing` lists pairs of `<route-path-glob> -> <destination-glob>`
- **AND** the guest calls `routing-control.update-target(route-path, destination)`
- **THEN** the host MUST find at least one pair where both globs match the respective arguments
- **OR** the host MUST return a permission-denied error
- **AND** the host MUST NOT allow an entry that supplies only a route-path glob (validation rejects it at manifest parse time)

#### Scenario: outbound-http url matches scheme://host/path only
- **WHEN** the deployment's `scopes.http` is `["https://api.example.com/v1/*"]`
- **AND** the guest calls `outbound-http.send-request("GET", "https://api.example.com/v1/users?token=abc", ...)`
- **THEN** the match MUST be performed against `https://api.example.com/v1/users` only (query string ignored)
- **AND** the call MUST succeed

### Requirement: Per-deployment Linker cache keyed on scope shape

The host SHALL cache `wasmtime::component::Linker` instances by `ScopeShape` (the normalized set of granted categories plus their pattern sets). Two deployments with the same scope shape SHALL share one `Arc<Linker<StoreData>>`. The cache SHALL be bounded by a configurable LRU limit (default 256) and SHALL expose `faas_linker_cache_hit_total` / `faas_linker_cache_miss_total` counters.

#### Scenario: Two deployments share a scope shape
- **WHEN** two deployments declare the same `scopes:` block (semantically equal after normalization)
- **THEN** the second instantiation MUST hit the linker cache and MUST NOT call any `add_to_linker` function

#### Scenario: Distinct shapes do not collide
- **WHEN** two deployments declare different scope sets
- **THEN** they MUST receive distinct `Linker` instances
- **AND** the cache MUST count two entries

#### Scenario: Cache eviction under LRU pressure
- **WHEN** the number of distinct scope shapes exceeds the LRU bound
- **THEN** the least-recently-used `Linker` MUST be evicted
- **AND** a subsequent instantiation of an evicted shape MUST rebuild the linker (cache miss counted)

### Requirement: Tenant scope context lives in StoreData, not in linker closures

The compiled `DeploymentScopes` (the `GlobSet`s) SHALL be stored in the per-instantiation `StoreData` and SHALL be read by host closures from the store. The cached `Linker` SHALL NOT capture any tenant-specific data in its registered closures. This invariant SHALL be documented in the scoping module.

#### Scenario: Same Linker, different stores, different scopes
- **WHEN** two deployments with the same `ScopeShape` but different secret pattern lists share a cached `Linker`
- **THEN** each guest's host call MUST see only its own deployment's scopes via its own `StoreData`
- **AND** a denial in one deployment MUST NOT affect the other

### Requirement: Capabilities bitmask and DeploymentScopes remain disjoint

The existing `Capabilities` bitmask (host hardware/feature gating, defined in `core-host/src/host_core/constants.rs`) and the new `DeploymentScopes` SHALL remain orthogonal. No code path SHALL consult one to derive the other. Routing decisions SHALL continue to use `Capabilities` only; per-call authorization SHALL use `DeploymentScopes` only.

#### Scenario: Capabilities-driven routing is unaffected
- **WHEN** a deployment requires `feature:ai-inference` for routing
- **THEN** the host MUST select a peer based on `capability_mask` matching as today
- **AND** the deployment's `scopes` MUST NOT participate in peer selection

#### Scenario: Scopes do not narrow capabilities
- **WHEN** a deployment's scopes do not include `vector:`
- **THEN** the deployment's advertised `Capabilities` MUST NOT change
- **AND** the deployment MAY still be routed to nodes advertising `feature:ai-inference`

### Requirement: WIT contract is unchanged

This change SHALL NOT modify `wit/tachyon.wit`. No interface signature, no resource shape, no world's import or export list SHALL change. No new world SHALL be introduced. The change SHALL be implemented entirely in the host-side linker construction and the per-interface host closures.

#### Scenario: wit/tachyon.wit content stable
- **WHEN** the change is implemented
- **THEN** `wit/tachyon.wit` MUST be byte-identical to its prior state (excluding non-semantic whitespace)
- **AND** no `world ... -with-scoping` or similar variant MUST be added

#### Scenario: Existing guest binaries continue to work
- **WHEN** a guest compiled before this change is loaded under `scopes: allow-all`
- **THEN** the guest MUST instantiate without rebuild
- **AND** all its calls MUST succeed as before

### Requirement: Manifest validation rejects malformed scopes at submission

Manifest validation SHALL reject manifests where `scopes` contains: unknown category keys; non-string pattern entries; `routing:` entries that omit either the route-path glob or the destination glob; patterns that do not compile under `globset` semantics. The rejection SHALL happen at manifest submission time, before any linker construction.

#### Scenario: Unknown category key
- **WHEN** the manifest contains `scopes.secrest: ["..."]`
- **THEN** validation MUST reject the manifest naming the unknown key

#### Scenario: routing entry without destination
- **WHEN** the manifest contains a `scopes.routing` entry with only a route-path glob
- **THEN** validation MUST reject the manifest with an error indicating destination is required

#### Scenario: Uncompilable glob pattern
- **WHEN** the manifest contains a pattern that fails to compile under `globset`
- **THEN** validation MUST reject the manifest naming the offending pattern

### Requirement: Observability for scope decisions

The host SHALL expose counters and structured logs for scope decisions: per-deployment counts of allow-all warnings, value-based denials, link-time denials, and linker cache hits/misses. Per-deployment denial counters SHALL be reportable via the existing telemetry interfaces.

#### Scenario: Denial increments a counter
- **WHEN** a guest's call to `secrets-vault.get-secret` is denied by scope
- **THEN** a per-deployment counter `faas_scope_denials_total{deployment, category}` MUST be incremented

#### Scenario: Sampled warning log on denial
- **WHEN** denials for a single deployment exceed a configured threshold
- **THEN** the host MUST emit a WARN log entry naming the deployment and the category
- **AND** the host MUST NOT emit a WARN entry for every denial (to avoid log floods from misbehaving guests)

