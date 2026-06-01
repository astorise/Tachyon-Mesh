## Context

`verify_integrity()` reads and cryptographically verifies the `integrity.lock`
manifest, returning a validated `IntegrityConfig`. The config is then passed to
`build_runtime_state` which constructs the live WASM router. The window between
verification and runtime construction is the natural injection point: the
signature is already checked (security invariant upheld), and the runtime hasn't
been built yet (no races or partial updates).

The same pattern applies to hot-reload: `reload_runtime_from_disk` reads and
verifies the manifest from disk, then calls `build_runtime_state`. Adding
`inject_feature_routes` at both sites ensures injected routes survive both cold
starts and live manifest updates pushed by the operator.

`GET /admin/nodes` already includes a self-entry built from
`self_registry_node(self_id, &state)`, which derives `active_systems` from
`runtime.config.routes` where `role == RouteRole::System`. Because
`inject_feature_routes` runs before `build_runtime_state`, the injected routes
are part of `runtime.config` and appear in `active_systems` without any
additional work.

## Goals / Non-Goals

**Goals:**
- Zero-configuration feature activation: install the binary, the features work.
- Idempotent: running `inject_feature_routes` on a manifest that already contains
  the routes is a no-op (supports re-seal workflows).
- Secure: injected routes are added *after* cryptographic verification, so the
  on-disk manifest signature is never invalidated.

**Non-Goals:**
- Injecting arbitrary routes from environment variables or config files.
- Auto-injection for feature-gated routes in worker nodes (only the primary
  node calling `serve_host` is in scope).
- Persisting injected routes back to `integrity.lock` (they live in memory only).

## Decisions

### D1 — Inject after verify, not in the manifest file
Modifying the manifest file would require re-signing, which needs the private
key and complicates the audit trail. Injecting into the in-memory `IntegrityConfig`
after verification is simpler, fully secure, and doesn't touch the signed artifact.

### D2 — `#[cfg(feature = "...")]` blocks, not runtime env checks
Feature availability is a compile-time fact. Using `cfg!` ensures dead code is
eliminated in binaries that don't include the feature, with zero runtime overhead.

### D3 — Route version `"1.0.0"` for injected routes
Matches the workspace-wide version bump from `1.1.0-alpha` to `1.0.0`. Using the
same version as the crate ensures the `has_drift` check in
`deployed_system_from_nodes` stays false for freshly-started nodes.

### D4 — Both `serve_host` and `reload_runtime_from_disk` call `inject_feature_routes`
A manifest POST from the client (e.g. after `import_faas_package`) triggers a
hot reload. The reloaded manifest comes from the client-written `integrity.lock`,
which may or may not include the injected routes depending on what the client
fetched. Calling `inject_feature_routes` on every reload guarantees consistency
regardless of the client's state.

## Risks / Trade-offs

- **Injected routes are not in `integrity.lock`**: if an operator reads
  `integrity.lock` directly, they will not see the injected routes. This is
  acceptable because the canonical source of truth for the *running* config is
  always `GET /admin/manifest`, not the file.
- **Version bump scope**: updating 64 `Cargo.toml` files in a single commit
  touches most of the workspace. The change is mechanical (string substitution
  only) and carries no semantic risk.

## Open Questions

None.
