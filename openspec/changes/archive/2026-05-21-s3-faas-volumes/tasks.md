## 1. Schema & Validation (core-host)

- [ ] 1.1 Add `VolumeType::S3` variant to the `VolumeType` enum in `domain_types.rs`
- [ ] 1.2 Add S3 URL validation in `integrity_config.rs` — reject `type: "s3"` on system routes, reject malformed `host_path` that doesn't match `s3://bucket/prefix`
- [ ] 1.3 Parse `host_path` into `(bucket, prefix)` in a helper used by the execution pipeline

## 2. S3 Volume Lifecycle (core-host)

- [ ] 2.1 Add `prepare_s3_volumes()` async fn — create per-invocation temp dir under `$TMPDIR/tachyon-s3-vol-<uuid>/`, download all objects from the S3 prefix using the existing `S3PersistenceBackend` store
- [ ] 2.2 Add `commit_s3_volumes()` async fn — upload all files from the temp dir back to the S3 prefix (only for `readonly: false`)
- [ ] 2.3 Add `cleanup_s3_volumes()` fn — remove the temp dir unconditionally after commit or on failure
- [ ] 2.4 Integrate `prepare_s3_volumes` / `commit_s3_volumes` / `cleanup_s3_volumes` into the guest execution pipeline in `guest_runtime.rs` (both legacy WASM and Component Model paths)
- [ ] 2.5 Wire the pre-opened temp dir into `preopen_route_volumes()` for S3 volumes (pass prepared dir path, respect `readonly` flag)

## 3. Admin API

- [ ] 3.1 Verify `POST /admin/manifest` already accepts the updated integrity.lock with S3 volumes (no schema breaking change — `host_path` is an existing string field)

## 4. MCP Tools (tachyon-mcp)

- [ ] 4.1 Implement `list_s3_volumes(route_path)` tool — GET /admin/manifest, filter volumes by `type: "s3"` for the given route, return list of `{bucket, prefix, guest_path, readonly}`
- [ ] 4.2 Implement `attach_s3_volume(route_path, s3_url, guest_path, readonly)` tool — load manifest, add S3 volume entry, POST updated manifest to /admin/manifest
- [ ] 4.3 Implement `detach_s3_volume(route_path, guest_path)` tool — load manifest, remove matching volume by `guest_path`, POST updated manifest to /admin/manifest
- [ ] 4.4 Register all three tools in the MCP server entry point

## 5. UI — Route Detail Volumes Panel (tachyon-ui)

- [ ] 5.1 Create `S3VolumeCard` component — displays S3 URL (bucket + prefix), guest mount path, RW/RO badge, Remove button
- [ ] 5.2 Create `VolumesPanel` component — lists all volumes for a route (Host, RAM, S3 cards), empty state with prompt when no volumes configured
- [ ] 5.3 Create `AddS3VolumeModal` component — fields: S3 URL (with inline `s3://bucket/prefix` validation), guest path, read-only toggle; submit calls `POST /admin/manifest`
- [ ] 5.4 Wire `VolumesPanel` into the route detail view with an "Add S3 Volume" action button
- [ ] 5.5 Add success/error toast notifications after add and remove operations
