# Tasks

## core-host — GET /admin/manifest

- [x] Add `admin_get_manifest_handler` in `integrity_config.rs`: returns
  `state.runtime.load().config` serialised as JSON (live in-memory config)
- [x] Register `GET /admin/manifest` alongside existing `POST /admin/manifest`
  in `app_runtime.rs`

## tachyon-client — workspace_root() with existence check

- [x] Rewrite `workspace_root()` to try candidates in order:
  `TACHYON_WORKSPACE_ROOT` → executable directory → compile-time path
- [x] Return the first candidate that **contains `integrity.lock`**, not merely
  the first candidate that was configured
- [x] Fall back to first candidate (meaningful NotFound error) when none match

## tachyon-client — get_topology_graph() cluster-first strategy

- [x] Extract `load_sealed_config_from_file()` helper with offline sentinel
- [x] When connected: call `GET /admin/manifest` to get live `IntegrityConfig`
- [x] On manifest endpoint error: fall back to local file (backward compat)
- [x] When not connected: read local `integrity.lock`
- [x] When file absent: return empty `TopologyGraphSpec` with status `"offline"`
