## Why

The current homelab deployment uses a rclone sidecar container that polls `/data → S3` every 30 seconds and an initContainer that restores from S3 on every pod start. This approach wastes resources (dedicated container, blind polling regardless of write activity), creates a 30-second data-loss window on pod crash, and couples Tachyon's persistence to an external binary. Tachyon already compiles `aws-lc-rs` for FIPS crypto; adding `object_store` (the S3 client used by Apache Arrow, DataFusion, Delta Lake) costs no new musl/TLS friction and lets Tachyon own its entire storage lifecycle natively.

## What Changes

- **New `s3-persistence` Cargo feature** in `core-host`: gates `object_store` with the `aws` feature set.
- **`S3PersistenceBackend`** struct: wraps an `ObjectStore` instance with a local root path and S3 prefix; exposes `restore()` and `flush_path()`.
- **Startup restore**: `serve_host()` calls `backend.restore()` before starting the HTTP server — replaces the rclone initContainer entirely.
- **Event-driven flush**: every mutating auth operation (`finalize_enrollment`, `finalize_login`, `consume_recovery_code`, `issue_pat`, `update_user`, `delete_user`, `upsert_group`, `delete_group`) calls `backend.flush_path(auth_state_dir)` after committing state — replaces the 30s polling sidecar.
- **Periodic background flush** (5 min) as a safety net for files written outside explicit auth paths (e.g. `tachyon.db`, `host-identity.key`).
- **`homelab.yaml` simplified**: remove `s3-restore` initContainer and `s3-sync` sidecar; add S3 env vars to the `core-host` container.

## Capabilities

### New Capabilities

- `s3-persistence`: native S3-backed persistence for Tachyon state (auth-state, core store, host identity, certs) using `object_store`.

### Modified Capabilities

- `github-actions`: `rust-ci` and feature-matrix add `--features s3-persistence` to the check step; `protobuf-compiler` already present.

## Impact

- **`core-host/Cargo.toml`**: `object_store = { version = "0.11", optional = true, features = ["aws"] }` under `s3-persistence` feature.
- **`core-host/src/persistence.rs`** (new): `S3PersistenceBackend`, `restore()`, `flush_path()`, `spawn_background_flush()`.
- **`core-host/src/host_core/entrypoint.rs`**: `serve_host()` reads S3 env vars, builds backend, calls `restore()`.
- **`core-host/src/auth.rs`**: every mutating public method gets a `flush_auth_state()` tail call via a thin wrapper or parameter.
- **`manifests/homelab.yaml`**: initContainer and s3-sync sidecar removed; S3 env vars added to core-host container.
- No WIT or guest API changes.
