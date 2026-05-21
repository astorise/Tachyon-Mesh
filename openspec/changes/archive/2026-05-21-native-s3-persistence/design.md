## Context

Tachyon's auth state is maintained by the `system-faas-authn` WASM component. `AuthManager` preopens `auth_state_dir` (derived from `TACHYON_INTEGRITY_MANIFEST`) as the WASM's working directory, so every user record, session, and token hash is a file under `/data/auth-state/`. The core store (`tachyon.db`) and host identity key live alongside at `/data/`. Currently an external rclone sidecar syncs these files blindly every 30 seconds.

## Goals / Non-Goals

**Goals:**
- Replace rclone initContainer + sidecar with a native Rust S3 client.
- Restore all state from S3 before serving requests.
- Flush auth state to S3 immediately after each mutating auth operation (zero polling for auth events).
- Keep a 5-minute background flush for non-auth files (`tachyon.db`, host identity key, certs).
- Compile cleanly on musl (Alpine) and glibc. `object_store` with `aws` feature has no native deps beyond TLS.

**Non-Goals:**
- Replacing `tachyon.db` (ReDB) with S3 as primary store — `/data` remains the source of truth; S3 is a durable backup/restore layer.
- Server-side encryption or bucket versioning — left to the homelab/cloud S3 config.
- Multi-pod concurrent writes — Tachyon is single-replica in the homelab; distributed locking deferred.

## Decisions

### D1: `object_store` v0.11 over `aws-sdk-s3`

`object_store` is 40k lines vs 2M lines for the full AWS SDK. It uses `reqwest` + `rustls` + `aws-lc-rs` (already in the dependency tree), compiles to musl without linker shims, and has a stable `put`/`get`/`list` API sufficient for file-sync semantics. The AWS SDK would pull in `tokio-rustls` ceremony and a large proc-macro surface we don't need.

### D2: Event-driven flush for auth, periodic for the rest

Auth operations are the most critical (admin account creation, token issuance) and the least frequent. Flushing synchronously after each write costs ≤50 ms per S3 PUT and ensures a pod crash never loses a committed auth event. The core store (`tachyon.db`) is a ReDB file that grows continuously; flushing it on every write would be expensive. A 5-minute background task is acceptable — losing 5 minutes of ReDB state on crash is survivable for a homelab; the auth records are the hard-to-reconstruct data.

### D3: Synchronous restore in `serve_host()` before bind

The server must not accept requests before state is restored. A sync restore (blocking tokio task) before `axum::serve` ensures correctness. Typical restore time for a few-KB auth-state is <500 ms on LAN; acceptable at startup.

### D4: `S3PersistenceConfig` from env vars, not integrity.lock

S3 credentials are infrastructure config, not sealed application config. Reading from env vars (`TACHYON_S3_ENDPOINT`, `TACHYON_S3_BUCKET`, `TACHYON_S3_ACCESS_KEY_ID`, `TACHYON_S3_SECRET_ACCESS_KEY`, `TACHYON_S3_PREFIX`) keeps the integrity manifest clean and lets the homelab inject them via Kubernetes Secret.

### D5: `flush_auth_state()` via wrapper in `AppState`, not inside `AuthManager`

`AuthManager` is a synchronous struct; S3 uploads are async. Rather than threading a `PersistenceBackend` Arc into every `spawn_blocking` call inside `AuthManager`, a thin async wrapper in the HTTP handler layer calls `flush_auth_state(state)` after each mutating handler returns. This keeps `AuthManager` single-responsibility and avoids async contamination of the blocking auth code.

## Risks / Trade-offs

- **S3 unavailable at startup** → `restore()` logs a warning and continues; Tachyon starts with empty state (same behavior as today when S3 is down). Operator must ensure S3 is reachable before the pod starts.
- **Partial flush on crash** → A file write to `/data` succeeded but the S3 PUT did not complete. On next restore, the old S3 version is used. Acceptable for homelab; mitigated by the background flush catching stragglers.
- **`object_store` feature flag on `s3-persistence`** → Default builds have zero S3 code linked in. The feature is additive.

## Migration Plan

1. Add `object_store` dep under `s3-persistence` feature.
2. Implement `persistence.rs` with `S3PersistenceConfig`, `S3PersistenceBackend`, `restore()`, `flush_path()`, `spawn_background_flush()`.
3. Wire restore into `serve_host()` (guarded by `cfg(feature="s3-persistence")`).
4. Add `flush_auth_state()` call to each mutating auth handler in `app_runtime.rs`.
5. Update `manifests/homelab.yaml`: remove initContainer + sidecar, add env vars to core-host.
6. Update CI: add `--features s3-persistence` to `cargo check` step.
7. Rebuild and redeploy homelab.

## Open Questions

- Should `flush_path()` be fire-and-forget (spawn) or awaited? → Awaited for auth (correctness), spawned for background (throughput). 
- Should we fsync `/data` before S3 PUT? → Yes for auth files; the WASM already calls `fs::write` which doesn't fsync. Adding `File::sync_all()` in `flush_path()` ensures durability.
