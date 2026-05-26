## Why

The Tachyon UI sidebar always renders all 16 navigation panels regardless of what is actually compiled into the cluster's nodes. Users see panels like "AI Orchestration", "Storage", or "Supply Chain" even when no node has the corresponding system compiled in its binary — creating confusion and dead-end navigation. Visibility must be driven by the `active_systems` each enrolled node reports in its capabilities, not by runtime hardware metrics or mounted volumes.

## What Changes

- New `ClusterFeatureSet` struct and `get_cluster_features()` Tauri command that aggregates cluster state into a flat set of boolean feature flags in one round-trip.
- New `clusterFeaturesStore` (TypeScript/Zustand) that fetches feature flags on connect and re-fetches on reconnect.
- `ComponentRoute` gains an optional `requires` field mapping to a `ClusterFeature` union type.
- `TachyonAppShellNav` filters routes to only those whose required feature is present (or routes with no requirement, which are always shown).
- Routes to unavailable panels via direct URL hash are redirected to the overview.

## Capabilities

### New Capabilities

- `cluster-feature-gating`: Backend command and frontend store that expose cluster feature availability, used to conditionally show/hide navigation panels.

### Modified Capabilities

- `app-shell`: Nav rendering now filters by available cluster features instead of listing all registered routes unconditionally.

## Impact

- **Rust** (`tachyon-client/src/lib.rs`, `tachyon-ui/src/main.rs`): new `ClusterFeatureSet` struct derived from `active_systems` slugs across enrolled nodes, new `get_cluster_features()` async fn, registered in Tauri invoke handler.
- **TypeScript** (`tachyon-ui/src/`): new `clusterFeaturesStore.ts`, updated `ComponentRegistry.ts` (adds `requires` field + `ClusterFeature` type), updated `TachyonAppShellNav.ts` (async filtering), updated `TachyonAppShell.ts` (redirect on unavailable route).
- **Source of truth**: `systems/manifest.toml` slug names drive the feature flag logic; no hardware metrics involved.
- No WIT/FaaS changes. No breaking changes to existing Tauri commands.
