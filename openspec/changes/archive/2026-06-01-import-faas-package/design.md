## Context

The Tachyon mesh already supports content-addressed WASM assets via
`push_asset_bytes` → `tachyon://sha256:<hex>` URIs that `resolve_guest_module_path`
resolves at runtime. The missing piece was a client-side operator for the
complete import lifecycle: extract archive → upload assets → patch manifest.

The live manifest is served by `GET /admin/manifest` as a raw `IntegrityConfig`
JSON object (not the on-disk `{config_payload, public_key, signature}` wrapper).
Several client functions were silently using the wrong deserialization target,
masking failures with a lockfile fallback.

## Goals / Non-Goals

**Goals:**
- Single-call `import_faas_package_bytes` that atomically uploads all WASMs and
  activates all routes from the embedded `manifest.json`.
- Fail fast (not silently) when not connected; surface actual HTTP errors.
- Expose the operation via MCP tool and Tauri command for UI and agent use.
- Ship a 9-route `guest-examples/manifest.json` covering all non-test guest WASMs.

**Non-Goals:**
- Rollback of a partial import (assets already uploaded are idempotent by hash).
- Multi-node fan-out (one node at a time, same as all other manifest operations).
- WASM cross-compilation or validation of module contents.

## Decisions

### D1 — Read live manifest via `get_manifest_config()`, not the lockfile
`GET /admin/manifest` returns `IntegrityConfig` directly. Using it (rather than
reading the local `integrity.lock`) means the import always starts from the
node's actual running state, even if the local file is stale or absent.
Fallback to lockfile is intentionally removed; the operation requires a live
connection.

### D2 — Content-addressed asset URIs in route targets
Imported routes store `tachyon://sha256:<hex>` in `targets[].module` rather than
a bare module name. The runtime already resolves these via `resolve_asset_uri`.
The validation layer needed a one-line fix to allow `/` in URIs that begin with
`tachyon://`.

### D3 — `patch_and_apply_manifest` increments `config_version`
The node rejects manifests whose `config_version` is not strictly greater than
the running version. The function now bumps the version before signing, matching
the pattern used by `apply_manifest_config`.

### D4 — Module name lookup uses both `_` and `-` forms
WASMs in the archive have underscore stems (`guest_call_legacy`). The manifest
references them with dashes (`guest-call-legacy`). The lookup map stores both
forms so either naming convention in `manifest.json` resolves correctly.

### D5 — `guest-examples/manifest.json` shipped inside the CI artifact
The tar-pack step already includes all `guest_*.wasm` files. Adding
`manifest.json` to the same archive lets the Workloads panel import WASMs and
routes in a single operation with no extra download step.

## Risks / Trade-offs

- **Partial import**: if the node rejects the manifest POST (e.g. validation
  error on one route), previously uploaded assets remain in the store but no
  routes are activated. Assets are content-addressed and idempotent, so a retry
  is safe. → Mitigation: return `ImportPackageResult` with `skipped_modules` so
  the operator knows what was skipped before the POST.

- **Large archives**: the entire tar.gz is read into memory in the Tauri process
  before upload. For the standard guest-examples package (~5 MB) this is fine;
  for hypothetical large model archives `push_large_model` (chunked upload) is
  the correct API. → Non-goal for this change.

- **Lockfile divergence after import**: `patch_and_apply_manifest` writes the
  updated manifest to the local `integrity.lock`. If the node is on a different
  machine, the local file will contain routes the remote node knows but that
  local tools may not have previously seen. This is the intended post-import
  state. → No mitigation needed.

## Migration Plan

All changes are additive. No existing endpoints or file formats are changed.
The three bug fixes (`get_manifest_config`, `patch_and_apply_manifest`,
`normalize_route_target`) are safe for existing callers: the manifest format fix
corrects a bug that was silently misbehaving, and the validation change only
broadens the accepted set of module names.

Rollout: deploy updated `core-host` binary first (accepts `tachyon://` URIs in
validation), then update `tachyon-client`/`tachyon-ui` binaries.

## Open Questions

None — all decisions were validated against the running homelab instance.
