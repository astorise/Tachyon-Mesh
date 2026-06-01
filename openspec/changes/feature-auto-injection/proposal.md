## Why

When an operator installs a `core-host` binary built with `--features ai-inference`
or `s3-persistence`, the corresponding system FaaS modules are present in
`guest-modules/` but their routes are absent from the manifest. The UI's
`has_ai` / `has_storage` feature flags derive from active system routes, so the
AI and Storage panels are invisible even though all WASMs are ready. Operators
had to manually toggle each route through the Systems panel and trigger a seal
before anything worked.

## What Changes

- **`inject_feature_routes`** (new function in `core-host/src/host_core/integrity_config.rs`): injects system routes for compiled-in features into an `IntegrityConfig` before it is applied to the runtime. Called from `serve_host` (after `verify_integrity`) and from `reload_runtime_from_disk`. Routes already present in the config are left untouched (idempotent).
  - `ai-inference` → `/system/model-broker`, `/system/ai-list-model`, `/system/ai-openai-adapter`
  - `s3-persistence` → `/system/s3-proxy`, `/system/storage-broker`
- **`get_cluster_features`** (client): `has_ai` and `has_storage` continue to derive from `active_systems` slugs reported by `GET /admin/nodes`, which are themselves built from `role=system` routes in the runtime config — so the injected routes make these flags true automatically.
- **Version bump `1.1.0-alpha` → `1.0.0`**: all workspace `Cargo.toml` files (64), `systems/manifest.toml` (39 entries), and the version string in `inject_feature_routes`.

## Capabilities

### New Capabilities

- `feature-auto-injection`: System routes for compiled-in feature flags are activated automatically at node startup, eliminating manual manifest sealing for standard feature bundles.

### Modified Capabilities

- `cryptographic-integrity`: `inject_feature_routes` modifies the `IntegrityConfig` after signature verification but before `build_runtime_state`; the signed payload on disk is not changed.

## Impact

- **`core-host/src/host_core/integrity_config.rs`**: ~40 lines added (`inject_feature_routes`).
- **`core-host/src/host_core/entrypoint.rs`**: one-line change wrapping `verify_integrity()` result.
- **`core-host/src/host_core/supervisors.rs`**: `reload_runtime_from_disk` passes config through `inject_feature_routes` before `build_runtime_state`.
- **64 `Cargo.toml` files + `systems/manifest.toml`**: version string change only.
