## Why

Today every FaaS guest that targets a given WIT world in `wit/tachyon.wit` gets unconditional access to **every** import declared by that world. A `faas-guest` can read any secret via `secrets-vault.get-secret`, open any `kv-partition.table` (including another tenant's), bridge to any IP via `bridge-controller`, and a `control-plane-faas` can rewrite any route via `routing-control.update-target`. The static WIT split (one world per FaaS family) is already good at distinguishing **categories of operations**, but it cannot express **which instances of those operations a given deployment is allowed to perform**. Splitting the worlds further to compensate is blocked by `feedback_wit_world_single_source` (the runtime guest-count cap is shared and forks stall migration). We need a per-deployment authorization layer that scopes imports without forking worlds.

## What Changes

- Add a new **deployment scope manifest** field (`scopes:`) consumed at guest instantiation in [guest_runtime.rs](core-host/src/host_core/guest_runtime.rs) that declares which imports — and which argument patterns within those imports — the deployment may use.
- Replace the single shared `Linker` for each world by a **per-deployment-shape `Linker` cache** keyed on the manifest scope set. The linker for a given shape registers only the `add_to_linker` calls authorized by that shape; everything else is absent, so any unauthorized import fails at **instantiation** with a link error and is unreachable at runtime.
- For imports that take an identifier as argument (string-based, e.g., `secrets-vault.get-secret(name)`, `routing-control.update-target(path, dest)`), wrap their host closures so that they capture a **pre-compiled `GlobSet`** of authorized patterns from the manifest. A non-matching argument returns the existing typed error variant (e.g., `authz-error::permission-denied`-style); no allocation, no global lookup.
- For imports that take **opaque identifiers behind resources** (`kv-partition.table::new(name)`, `vector.create-index(spec)`, `bridge-controller.create-bridge(config)`), the scope check fires only at **resource construction** and the handle then carries the validated context; all subsequent methods on that handle skip the check entirely.
- **BREAKING for operators**: deployment manifests now require a `scopes:` block. A migration default of "preserve current behavior" (`scopes: allow-all`) ships in the same change so existing deployments keep working until operators tighten them.
- The change is purely additive to `wit/tachyon.wit` (no signature changes, no new worlds). Existing guest builds keep working without rebuild as long as they fall under `allow-all`.

## Capabilities

### New Capabilities

- `faas-import-scoping`: declarative, per-deployment authorization layer that filters which WIT imports of `wit/tachyon.wit` worlds are wired into the wasmtime `Linker` for a given guest instantiation, and validates argument patterns / resource constructor arguments against a compiled scope set captured in each host closure.

### Modified Capabilities

_None — this change introduces a new orthogonal layer. It deliberately does not modify any existing WIT signature, world, or `Capabilities` bitmask semantics (those remain host-side feature flags). The existing `Capabilities` from [constants.rs](core-host/src/host_core/constants.rs) and the new `DeploymentScopes` are complementary: hardware/feature capabilities gate which **host** can run a route; scopes gate which **arguments and resources** a guest may use._

## Impact

- **Code**:
  - [guest_runtime.rs](core-host/src/host_core/guest_runtime.rs): linker construction moves from "build once per world" to "build once per scope-shape, cached by shape hash".
  - [component_hosts.rs](core-host/src/host_core/component_hosts.rs): deployment record gains a `scopes: DeploymentScopes` field; instantiation reads it.
  - New module `core-host/src/host_core/scoping.rs` (or similar): `DeploymentScopes`, `ScopeShape`, `LinkerCache`, the per-interface `add_to_linker_scoped` helpers.
  - Host closures for `secrets-vault`, `kv-partition`, `bridge-controller`, `routing-control`, `vector`, `training`, `storage-broker`, `outbound-http`, `outbox-store`, `graph` updated to consume the scope context captured in `StoreData`.
- **APIs (operator-facing)**: manifest schema gains `scopes:` block. Documented patterns: `secrets:db/*`, `kv:tenant-X/*`, `bridge:10.0.0.0/8`, `http:https://api.example.com/*`, `routing:/system/*`.
- **`wit/tachyon.wit`**: no change (per `feedback_wit_world_single_source`).
- **Guest SDKs / Rust guests**: no rebuild required (no WIT signature change).
- **Performance**: zero per-call overhead for imports absent from the linker (unreachable). ~30 ns per call for value-based imports with a compiled `GlobSet`. One-time ~50–100 µs cost per *new* scope shape (cached afterwards). See `design.md` §Performance.
- **Tests**: existing FaaS integration tests must keep passing under `scopes: allow-all`. New tests verify (a) a guest without scope for `bridge-controller` fails to instantiate; (b) a guest with `secrets:db/*` rejects `auth/master`; (c) a guest with `kv:tenant-A` cannot open `tenant-B`.
- **Security posture**: removes ambient authority from FaaS guests by default — closes the cross-tenant read primitive on `kv-partition`, the arbitrary-IP outbound primitive on `bridge-controller`, and the route-hijack primitive on `routing-control` for `control-plane-faas`.
