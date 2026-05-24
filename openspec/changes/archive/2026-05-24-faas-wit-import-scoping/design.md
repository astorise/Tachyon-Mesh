## Context

`wit/tachyon.wit` (package `tachyon:mesh@1.1.0`) declares ~14 interfaces (`handler`, `udp-handler`, `websocket`, `custom-metrics`, `telemetry-reader`, `scaling-metrics`, `vector`, `training`, `storage-broker`, `outbound-http`, `outbox-store`, `bridge-controller`, `routing-control`, `secrets-vault`, `graph`, `kv-partition`) and six worlds: `faas-guest`, `udp-faas-guest`, `websocket-faas-guest`, `system-faas-guest`, `background-system-faas`, `control-plane-faas`. The static split by world is already in place: e.g. `faas-guest` does not import `routing-control`, `udp-faas-guest` only imports `secrets-vault`, `control-plane-faas` does not import `vector`/`training`.

However, in [guest_runtime.rs](core-host/src/host_core/guest_runtime.rs#L295-L345) the host today calls every `tachyon::mesh::<iface>::add_to_linker::<...>` declared by a world unconditionally. Once linked, a guest can call any imported function on **any argument**:

- `secrets-vault.get-secret(name)` accepts any `name` — no per-deployment ACL.
- `kv-partition.table::new(name)` accepts any `name` — cross-tenant reads possible.
- `bridge-controller.create-bridge(config)` accepts any `client-a-addr`/`client-b-addr`.
- `routing-control.update-target(route-path, destination)` (in `control-plane-faas`) can rewrite any route.
- `outbound-http.send-request(method, url, ...)` can hit any URL.

The existing `Capabilities` bitmask in [constants.rs](core-host/src/host_core/constants.rs#L178-L215) is **about the host**, not the guest: it gates whether a node advertises CUDA, websockets, HTTP/3, etc. It is the wrong layer to express "guest X may only touch secrets matching `db/*`".

Two prior conversations frame the constraints:

1. `feedback_wit_world_single_source`: we cannot fork `-with-X` variant worlds because the FaaS runtime cap on concurrently loaded guests is shared. So adding a new world per-tenant or per-scope is off the table.
2. The architecture conversation (this change's origin): the consensus was to keep WIT as the **category** boundary and add a runtime authorization layer for **instances**, ideally pushed as early as possible — link/instantiation time rather than per-call.

## Goals / Non-Goals

**Goals:**

- Make unauthorized imports **structurally unreachable** at link time, not just denied at call time.
- For value-based authorization that can only be checked at runtime, make the check **O(1)–O(log n) and tenant-local** (closure-captured `GlobSet`), with no global lookup, no allocation on the hot path.
- Cache `Linker` instances by **scope shape** so the construction cost is paid once per distinct manifest pattern set, not per instantiation.
- Preserve `wit/tachyon.wit` as the single source of truth; no new worlds, no signature changes.
- Provide a default-allow migration mode so existing deployments keep functioning while operators progressively tighten scopes.
- Tie scope checks to **resource constructors** wherever the WIT already exposes a resource (`kv-partition.table`, `vector` indexes by name, `graph.workspace-graph`, `websocket.connection`), so per-method overhead is zero after handle creation.

**Non-Goals:**

- Modifying any WIT interface or world (no signature drift; respects `feedback_wit_world_single_source`).
- Replacing or unifying with `Capabilities` (host hardware features) — these stay orthogonal.
- Adding identity/authentication of the caller of a guest's exported handler. That is `authn`/`authz` territory and out of scope here. This change scopes what the **guest** can call into the host, not who calls into the guest.
- Per-call accounting/quotas (rate limiting, metering). Those exist elsewhere and remain separate.
- Building a policy DSL — the manifest carries flat patterns (`secrets:db/*`, `kv:tenant-X/*`, ...). Anything richer would belong in a future change.

## Decisions

### D1. Filter `add_to_linker` per deployment instead of forking worlds

**Decision.** Construct the `wasmtime::component::Linker` per deployment scope shape. Each `tachyon::mesh::<iface>::add_to_linker` call is conditional on the scope set granting that interface. Imports denied at the interface level are **absent from the linker**; instantiation of a guest that uses them fails with a wasmtime link error.

**Why.** Forking worlds (e.g. `faas-guest-without-bridge`) is ruled out by `feedback_wit_world_single_source`: a parallel world stalls the shared guest-count cap during migration. Filtering at link time is the next-most-static boundary available within a single world — and is in fact stronger than a world fork because it can be parameterized per deployment without changing the WIT contract. Spin and wasmCloud both use this pattern.

**Alternatives rejected.**

- **Per-call check inside every host closure unconditionally**: simpler to write but pays a global lookup for every WASM↔host trip, including for guests that should not be able to call the import at all. The wasmtime linker is the right place for "can this function even be invoked?" — value-based checks should only handle "is this *specific argument* allowed?".
- **A new `policy` WIT interface that the host calls into**: would let guests *see* policy, defeating the goal of making unauthorized imports unreachable. Also adds a host↔guest round-trip per check.

### D2. Cache `Linker` instances by scope-shape hash

**Decision.** Introduce a `LinkerCache: DashMap<ScopeShapeHash, Arc<Linker<StoreData>>>`. A `ScopeShape` is the normalized set of `(interface, pattern_set)` enabled by a manifest. Two deployments with the same shape share one linker. The `Component` is cached separately (already true today).

**Why.** Building a linker for a full world is on the order of 50–100 µs (one `add_to_linker` per interface + closure setup). Doing that per instantiation in a hot scale-out path would dominate cold-start cost. Caching by shape amortizes the cost: in typical fleets there are dozens of distinct manifests, not thousands, so the cache stays small and warm.

**Alternatives rejected.**

- **One linker per deployment**: O(deployments) linkers; redundant when most deployments share scope shapes.
- **One linker globally, with per-call dispatch on store data**: brings the runtime-check problem back into every host import even when the import is universally denied.

### D3. Tenant context lives in `StoreData` and is captured by host closures

**Decision.** Each guest instantiation builds a fresh `StoreData` containing the resolved `DeploymentScopes` (compiled `GlobSet` per scope category). Host closures registered in the linker take `&mut StoreData` (already the case for the value-based imports today) and read scopes from there. The closure does not hold the scope set itself — it reads it from the store on each invocation. The cached linker therefore stays tenant-agnostic; only the store is tenant-specific.

**Why.** Wasmtime linkers are designed to be shared across stores. Capturing tenant data inside a closure stored in the linker would force one linker per tenant (defeats D2). Reading from `StoreData` is a pointer-chase already paid by every host call (the store reference is the first argument).

**Trade-off.** Per-call check cost for value-based imports is not zero — but it is `store.scopes.secrets.is_match(name)` against a precompiled `GlobSet`, ~20–50 ns. Acceptable given the WASM↔host trip is already ~200 ns – 2 µs.

### D4. Resource-bound scoping for handle-creating imports

**Decision.** For `kv-partition.table::new(name)`, `vector.create-index(spec)`, `bridge-controller.create-bridge(config)`, `graph.workspace-graph::new(name)`: the check fires in the **constructor closure** and the resulting wasmtime resource handle is implicitly bound to the validated argument. All subsequent methods on that handle (`get`, `set`, `delete`, `batch-set`, etc.) **do not re-check**; the host implementation trusts the handle.

**Why.** This pattern is the highest-leverage win because `kv-partition` is the only interface plausibly called in tight loops by user code. Pushing the single check to construction makes the steady-state cost identical to the unscoped baseline.

**Alternatives rejected.**

- **Re-check the validated name on every method**: 30–50 ns per call × millions of ops/s = measurable in benchmarks; not justified when the constructor already enforced the boundary.
- **Encode the tenant inside the resource ID by name mangling**: brittle, leaks naming into the WIT contract, and breaks if the WIT ever changes the resource type.

### D5. Scope categories and pattern syntax

**Decision.** Scopes are flat strings of the form `<category>:<pattern>`. Categories are bound to WIT interfaces:

| Category | Gates | Check timing |
|---|---|---|
| `secrets:` | `secrets-vault.get-secret(name)` | per call (value-based) |
| `kv:` | `kv-partition.table::new(name)` | per `table::new` (handle-bound) |
| `vector:` | `vector.create-index`, `upsert`, `search` (by `index-name`) | per call (value-based on name) |
| `training:` | `training.submit-training-job(job)` (by `dataset.volume-alias`) | per call (value-based) |
| `bridge:` | `bridge-controller.create-bridge(config)` (by `client-a-addr`, `client-b-addr`) | per `create-bridge` (handle-bound) |
| `routing:` | `routing-control.update-target(route-path, destination)` | per call (value-based on `route-path` AND `destination`) |
| `http:` | `outbound-http.send-request(method, url, ...)` | per call (value-based on `url`) |
| `outbox:` | `outbox-store.claim-events`, `ack-event` (by `db-url` + `table`) | per call |
| `storage:` | `storage-broker.enqueue-write`, `snapshot-volume`, `restore-volume` | per call (value-based on `path` / `volume-id`) |
| `graph:` | `graph.workspace-graph::new(name)` | per `workspace-graph::new` (handle-bound) |
| `outbound-http:`, `bridge-controller:`, `routing-control:` (no pattern) | interface-level allow/deny without arg pattern | linker-time only |

Patterns use `globset` semantics: `*` matches a path segment, `**` matches across segments. Empty pattern (`secrets:`) denies all in that category. Absence of a category = interface-level deny (not even linked).

A wildcard `scope: allow-all` is allowed in the manifest for migration only; emits a warning in logs and a metric.

**Why glob over regex.** Globs cover the use cases (tenant-prefix, URL-prefix, path-prefix) without the foot-gun surface of regex (catastrophic backtracking on attacker input). The `globset` crate compiles to an `AhoCorasick`-backed set, so matching is linear in argument length.

### D6. Manifest schema addition

**Decision.** The deployment manifest (whatever today drives [component_hosts.rs](core-host/src/host_core/component_hosts.rs)) gains:

```yaml
scopes:
  secrets: ["db/prod/*", "config/feature-flags"]
  kv: ["tenant-a/*"]
  http: ["https://api.stripe.com/*", "https://hooks.slack.com/*"]
  bridge: []   # explicit empty = deny all bridge creates
  # absent categories: interface not linked at all
```

Plus a migration default:

```yaml
scopes: allow-all
```

Equivalent to all categories with `["**"]`. Emits a warning at instantiation.

**Why explicit empty vs. absent.** Operators need a way to say "this guest needs the interface (so its world links cleanly) but should not be able to do anything with it" — e.g. a system FaaS may want to keep `bridge-controller` linked for telemetry coherence but disabled by policy. Equally important: `absent` gives the strongest guarantee (link error) while `empty` gives a typed runtime error the guest can handle.

### D7. Error surface to the guest

**Decision.** Value-based denials map to the existing typed error variant of each interface:

- `secrets-vault.get-secret` → `error::permission-denied` (already in the WIT).
- `kv-partition.table::new` → `result::err("permission denied: kv:<name> not granted")`.
- `outbound-http.send-request` → `result::err("permission denied: http:<url> not granted")`.

We do **not** add a new shared error type to the WIT. Reason: keeps `wit/tachyon.wit` untouched per D-overall and `feedback_wit_world_single_source`. The string body is human-readable; structured variants stay per-interface.

Link-time denials (interface absent) surface as a wasmtime instantiation error, logged with the missing import name and the offending deployment id. The deployment fails to start; the supervisor reports the link error in the existing failure path.

### D8. Capabilities bitmask vs. scopes — disjoint layers

**Decision.** Keep them entirely separate. `Capabilities` (in `constants.rs`) gates **host-fitness** for a route (does this node have CUDA, websockets, HTTP/3?). `DeploymentScopes` gates **guest-authorized arguments** of host imports. They never read each other.

**Why.** Conflating them would either (a) inflate the `u64` mask beyond its 64-bit capacity (we already have 11 flags out of 64; tenant-scoping could explode that) or (b) leak tenant identity into routing decisions. They answer different questions and should not share storage.

## Risks / Trade-offs

- **[Risk]** A manifest typo (`secrest: ...`) silently leaves the category as "absent" → interface unlinked → guest fails to instantiate with an opaque wasmtime error.
  → **Mitigation.** Validate manifest at submission time against the known category set; reject unknown keys with a clear error before reaching the linker.

- **[Risk]** Pre-existing deployments without a `scopes:` block break on upgrade.
  → **Mitigation.** Default to `scopes: allow-all` when the field is missing **and** emit a warning + metric. Operators can grep for the metric to find unscoped deployments and tighten them. After a deprecation window we can change the default.

- **[Risk]** `LinkerCache` grows unbounded if operators generate many distinct scope shapes (e.g., one per deployment).
  → **Mitigation.** LRU bound on the cache (default 256 entries); cache miss only re-builds the linker (~100 µs). Logs `linker_cache_hit/miss` counters.

- **[Risk]** Resource-bound scoping (D4) silently weakens if the WIT ever changes a handle method to take a *second* name argument referencing another resource.
  → **Mitigation.** Document the invariant in `core-host/src/host_core/scoping.rs`. Add a CI lint or audit comment in `wit/tachyon.wit` near each resource: "scope-bound at constructor; methods must not take cross-resource identifiers without re-scoping".

- **[Risk]** `routing-control.update-target` takes both `route-path` and `destination` — operators may scope only `route-path` and forget `destination`, allowing a control-plane guest to redirect an authorized route to an attacker-controlled host.
  → **Mitigation.** The `routing:` scope is treated as a tuple match: every entry is `routing:<route-path-glob>->!<destination-glob>` OR a category sub-key. Enforce both in the host closure. Validation rejects single-glob entries for `routing:`.

- **[Trade-off]** Operators now write more manifest. The boilerplate is real, but the alternative (ambient authority on a multi-tenant runtime) is not viable.

- **[Trade-off]** Per-call check on `outbound-http.send-request` adds ~50–100 ns per HTTP call (URL globset match) compared to today. Negligible vs. the outbound HTTP latency itself.

## Migration Plan

1. **Phase 1 — code lands, default allow-all.** Ship the `DeploymentScopes` parser, `LinkerCache`, and per-interface scoped closures. Existing manifests without `scopes:` resolve to `allow-all`. Log a `faas_scopes_allow_all_total` counter per deployment.

2. **Phase 2 — operator tightening.** Operators add `scopes:` blocks to manifests for their FaaS. The CLI / UI shows the `allow-all` counter so they can prioritize.

3. **Phase 3 — opt-in strict default.** A node-level setting (`require_scopes: true`) refuses to instantiate deployments without explicit scopes. Rolled out per cluster.

4. **Phase 4 — flip the default** (separate, future change). Once telemetry shows ~zero `allow-all` deployments, change the default to "deny when absent". Behind its own openspec change, not this one.

**Rollback.** Each phase is independently reversible by config. Code-level rollback: a feature flag `cfg(feature = "faas-import-scoping")` could gate the new linker path, falling back to the existing unscoped path — but the recommended rollback is to set every manifest to `allow-all`, which restores prior behavior with the new code in place. No data migration; no on-disk format change.

## Open Questions

- **OQ1.** Should `outbound-http` `url` patterns include or exclude query strings? Excluding is safer (operators reason about hosts, not query strings), but breaks legitimate use cases where the path encodes the resource. **Proposed default: glob on `scheme://host/path` only, query string ignored for matching.**

- **OQ2.** Where does the per-deployment `DeploymentScopes` get persisted? It rides in the deployment manifest, but the resolved compiled form (the `GlobSet`s) is rebuilt per instantiation. Should we cache compiled forms keyed by manifest hash to skip recompilation on warm restarts? **Defer — only worth it if compile cost shows up in profiles.**

- **OQ3.** Audit logging: should every value-based denial be logged at WARN with the offending pattern? Risk of log volume from a misbehaving guest. **Proposed default: WARN-sample at 1/100 + always-increment a per-deployment counter; full log only when counter crosses a threshold.**

- **OQ4.** Interaction with hot-reload (`active-hot-reload` capability): when an operator updates scopes on a running deployment, do we reinstantiate? **Proposed: hot-reload of scopes triggers a fresh `Linker` lookup and store rebuild for new instances; existing instances finish in-flight requests under the old scopes and are recycled.** Needs confirmation with the hot-reload owner.
