# FaaS Import Scoping

Per-deployment authorization for WIT imports. Each FaaS deployment declares which interfaces and resources it is permitted to use. Unauthorized interfaces are absent from the guest's linker (link-time denial) or blocked at the first API call (runtime denial for value-based arguments).

## `scopes:` manifest syntax

Add a top-level `scopes:` block to your `integrity.lock` route entry:

```yaml
scopes:
  secrets:
    - "db/prod/*"
    - "db/staging/*"
  kv:
    - "tenant-a/**"
  vector:
    - "embeddings-*"
  http:
    - "https://api.example.com/**"
  routing:
    - "/api/v2/* -> /internal/v2/*"
  outbox:
    - "postgres://db.internal/*/events"
  storage:
    - "uploads/**"
  training:
    - "datasets/tenant-a/**"
  bridge:
    - "10.0.0.*:*"
  graph:
    - "workspace-a"
```

### Categories

| Category   | Checked against                                  | Check point          |
|------------|--------------------------------------------------|----------------------|
| `secrets`  | Secret name passed to `get-secret`               | per call             |
| `kv`       | Table name passed to `table::new`                | constructor (handle-bound) |
| `vector`   | Index name on every vector operation             | per call             |
| `training` | `job.dataset.volume-alias`                       | per call             |
| `bridge`   | Both peer addresses in `create-bridge`           | per call             |
| `routing`  | `route-path → destination` pair                  | per call             |
| `http`     | URL scheme+host+path (query string stripped)     | per call             |
| `outbox`   | `<db-url>/<table>`                               | per call             |
| `storage`  | File path, volume-id                             | per call             |
| `graph`    | Workspace name passed to `workspace-graph::new`  | constructor (handle-bound) |

### Pattern syntax

Patterns use [globset](https://docs.rs/globset) semantics:

- `*` matches within a single path segment (no `/`)
- `**` matches across segments including `/`
- An empty list (`secrets: []`) denies all access to that category at runtime

### Routing tuples

The `routing:` category uses `route-path -> destination` pairs (space-arrow-space):

```yaml
routing:
  - "/api/v2/* -> /internal/v2/*"
  - "/webhooks/** -> https://hooks.example.com/**"
```

Both the route-path and the destination must match their respective globs for a call to be allowed. A `routing:` entry that is missing `->` is rejected at manifest submission.

### Absent categories

A category not present in `scopes:` means the corresponding WIT interface is **not linked at all** — the guest component cannot call any function in that interface. Attempting to instantiate a component that imports the interface will produce a wasmtime link error, not a runtime denial.

### Migration default: `allow-all`

If `scopes:` is omitted, the deployment resolves to `allow-all` and the host logs a warning plus increments `faas_scopes_allow_all_total{deployment}`. This preserves backward compatibility during migration.

```yaml
scopes: allow-all   # explicit allow-all; emits the same warning
```

To reject allow-all deployments at submission time, set `requireScopes: true` at the node level.

## Link-time vs. runtime enforcement

| Enforcement point | When                         | Guest sees                          |
|-------------------|------------------------------|-------------------------------------|
| Link-time         | Guest instantiation          | wasmtime error naming the missing import |
| Runtime (handle)  | Resource constructor         | `Err("permission denied: …")`       |
| Runtime (per-call)| Every value-based call       | WIT error type or string error      |

Handle-bound resources (`kv-partition.table`, `graph.workspace-graph`) are checked once at construction; subsequent method calls on the handle are not re-checked.

## Migration plan

### Phase 1 — code lands, default allow-all (current)

Scoping infrastructure is live. Deployments without `scopes:` resolve to `allow-all`. The `faas_scopes_allow_all_total{deployment}` metric identifies unscoped deployments that need attention.

### Phase 2 — operator tightening

Add `scopes:` blocks to manifests, starting with deployments showing the most denials in the `faas_scope_denials_total{deployment,category}` metric. Iterate until `faas_scopes_allow_all_total` reaches zero for your cluster.

### Phase 3 — opt-in strict default

Set `requireScopes: true` in the node configuration to reject manifests without explicit scopes at submission time. Roll out per cluster.

### Phase 4 — flip the default (future change)

Once telemetry shows zero `allow-all` deployments across the fleet, the default will change to deny-when-absent in a separate openspec change. No action required from operators who have completed Phase 2.

**Legacy opt-out.** `require_scopes: false` remains a supported, permanent setting after the fleet default changes — it is not a deprecated flag scheduled for removal. Clusters that cannot or will not scope every deployment (fixed-manifest air-gapped clusters, single-tenant nodes where the authorization boundary doesn't matter) may pin `require_scopes: false` explicitly in their own manifest indefinitely.

**Rollback at any phase:** set `scopes: allow-all` on all manifests to restore prior behavior without downtime. No data migration, no on-disk format change.
