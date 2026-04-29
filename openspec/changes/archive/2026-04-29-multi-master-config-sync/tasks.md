# Implementation Tasks

## Phase 1: API & Versioning
- [x] `IntegrityConfig::config_version: u64` already added in Wave 0; the
      manifest parser honours it through the existing `IntegrityManifest`
      flow with no extra parsing required.
- [x] `POST /admin/manifest` is now exposed on every node (gated by the
      existing `admin_auth_middleware`); any node accepting the call
      becomes the origin for the new version.

## Phase 2: Gossip Integration (host-side)
- [x] New `config_update_outbox` redb table.
- [x] Successful manifest writes append a
      `ConfigUpdateEvent { version, checksum, origin_node_id, ts_ms }` row
      so the gossip bridge can broadcast it to peers.
- [x] Atomic write semantics: temp file + rename, picked up by the
      `notify`-based file watcher, which already handles the actual
      runtime swap and in-flight request drain.

## Phase 3: P2P Pull Logic
- [ ] The peer-pull side (gossip broadcast of the outbox events + secure
      pull from origin) ships in Session C alongside
      `system-faas-mesh-overlay`. The outbox writes are durable and
      forward-compatible with that integration; nothing about the schema
      or event format needs to change.

## Phase 4: Validation
- [x] Three unit tests:
  - `admin_manifest_update_accepts_higher_version_and_emits_outbox_event`
  - `admin_manifest_update_rejects_rollback` (409 on stale version)
  - `admin_manifest_update_rejects_tampered_signature` (400 on bad sig)
- [ ] Multi-node end-to-end test left for Session C, when the gossip
      bridge can complete the propagation.
