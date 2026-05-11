# Proposal: Topology Data Source — Live Cluster via GET /admin/manifest

## Problem

The topology canvas delivered in `2026-05-11-topology-canvas-live-zoom-compact`
read `integrity.lock` from the local filesystem via `workspace_root()`. This
caused two separate issues:

### 1 — Path resolution was fragile

`workspace_root()` used `env!("CARGO_MANIFEST_DIR")` as a compile-time
fallback, but the Tauri setup hook overrode it unconditionally with
`app_local_data_dir()` (e.g. `C:\Users\...\AppData\Local\tachyon-mesh\data`).
In development, `integrity.lock` lives in the Git workspace root, not in the
app data dir, so the override always produced a "file not found" → the UI fell
back to the offline sample topology on every launch.

### 2 — Local filesystem access is architecturally wrong

Reading `integrity.lock` from disk is only possible when the client runs on the
same machine as the node. In any multi-node or remote-admin scenario, the file
is simply not accessible. The topology should reflect the **live cluster state**,
not a potentially stale copy on the operator's workstation.

## What Changes

### `core-host` — `GET /admin/manifest`

New handler `admin_get_manifest_handler` added to `integrity_config.rs`.  It
returns the active `IntegrityConfig` serialised as JSON directly from
`state.runtime.load().config` — the in-memory config object that is swapped
atomically on every hot-reload.  The handler is registered on the existing
`/admin/manifest` route alongside the existing `POST` (update) handler.

This endpoint is the canonical source of truth for the deployed topology: it
reflects the exact running state, is always accessible over the same authenticated
HTTP channel used by every other admin operation, and works from any machine with
network access to the node.

### `tachyon-client` — `workspace_root()` with existence check

`workspace_root()` was rewritten to try candidate paths in priority order
and return the **first one that actually contains an `integrity.lock`**:

1. `TACHYON_WORKSPACE_ROOT` env var (if set)
2. Directory of the running executable
3. Compile-time `CARGO_MANIFEST_DIR` parent (development fallback)

This eliminates the previous footgun where the Tauri setup env-var override
would shadow a valid workspace path.

### `tachyon-client` — `get_topology_graph()` cluster-first strategy

`get_topology_graph()` now follows a three-tier resolution:

1. **Connected → `GET /admin/manifest`** — fetches `IntegrityConfig` directly
   from the running node. No filesystem access required. Works from any machine.
2. **Connected but endpoint unreachable** — falls back to local `integrity.lock`
   for backward compatibility with older nodes that don't expose the GET handler.
3. **Not connected** — reads local `integrity.lock`; returns empty nodes (offline
   sample fallback in the UI) when the file is absent.

The helper `load_sealed_config_from_file()` encapsulates the local-file path
and propagates a sentinel error (`__offline__`) that `get_topology_graph()`
translates into an empty `TopologyGraphSpec` with `status: "offline"`.
