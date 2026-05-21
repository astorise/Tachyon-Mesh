## 1. Cargo dependency

- [ ] 1.1 Add `object_store = { version = "0.11", optional = true, features = ["aws"] }` to `core-host/Cargo.toml` under `s3-persistence` feature
- [ ] 1.2 Add `s3-persistence` feature definition listing `dep:object_store`
- [ ] 1.3 Run `cargo check -p core-host --features s3-persistence` to verify deps resolve

## 2. persistence.rs module

- [ ] 2.1 Create `core-host/src/persistence.rs` (gated `#[cfg(feature = "s3-persistence")]`)
- [ ] 2.2 Implement `S3PersistenceConfig::from_env()` reading `TACHYON_S3_ENDPOINT`, `TACHYON_S3_BUCKET`, `TACHYON_S3_ACCESS_KEY_ID`, `TACHYON_S3_SECRET_ACCESS_KEY`, `TACHYON_S3_PREFIX`
- [ ] 2.3 Implement `S3PersistenceBackend::new(config, local_root)` building `object_store::aws::AmazonS3`
- [ ] 2.4 Implement `restore(&self) -> Result<()>`: list bucket objects under prefix, download each to local_root
- [ ] 2.5 Implement `flush_path(&self, path: &Path) -> Result<()>`: walk path recursively, PUT each file to S3 with matching key
- [ ] 2.6 Implement `spawn_background_flush(backend, interval)`: tokio task flushing local_root every 5 min
- [ ] 2.7 Add unit tests for `from_env()` and path key derivation (no real S3 needed)

## 3. Startup restore

- [ ] 3.1 In `entrypoint.rs::serve_host()`, after `open_core_store_for_manifest`, build `S3PersistenceConfig::from_env()` under `cfg(s3-persistence)`
- [ ] 3.2 If config present, call `backend.restore().await` before `axum::serve`
- [ ] 3.3 Spawn background flush task

## 4. Event-driven auth flush

- [ ] 4.1 Add `flush_auth_state(state: &AppState)` async helper in `app_runtime.rs` that calls `backend.flush_path(auth_state_dir)` under `cfg(s3-persistence)`
- [ ] 4.2 Add `flush_auth_state` call to `finalize_enrollment_handler` (POST /auth/signup/finalize)
- [ ] 4.3 Add `flush_auth_state` call to `finalize_login_handler` (POST /auth/login/finalize)
- [ ] 4.4 Add `flush_auth_state` call to `consume_recovery_code_handler`
- [ ] 4.5 Add `flush_auth_state` call to `issue_pat_handler`
- [ ] 4.6 Add `flush_auth_state` call to `update_user_handler`
- [ ] 4.7 Add `flush_auth_state` call to `delete_user_handler`
- [ ] 4.8 Add `flush_auth_state` call to `upsert_group_handler` and `delete_group_handler`
- [ ] 4.9 Add `flush_auth_state` call to `regenerate_account_security_handler`

## 5. AppState wiring

- [ ] 5.1 Add `s3_backend: Option<Arc<S3PersistenceBackend>>` field to `AppState` under `cfg(s3-persistence)`
- [ ] 5.2 Populate field in `build_app_state()` from the backend built in `serve_host()`

## 6. manifests/homelab.yaml

- [ ] 6.1 Remove `s3-restore` initContainer
- [ ] 6.2 Remove `s3-sync` sidecar container
- [ ] 6.3 Add S3 env vars to `core-host` container sourced from `tachyon-s3-creds` Secret
- [ ] 6.4 Keep `volumes: emptyDir` for `/data` (still needed as working directory)

## 7. CI

- [ ] 7.1 Add `cargo check -p core-host --features s3-persistence` step to `rust-ci` job after existing ai-inference check
- [ ] 7.2 Add `--features s3-persistence` matrix entry to `feature-matrix-tests` job
- [ ] 7.3 Add Docker `-s3` variant to `publish-docker-images` matrix (optional, lower priority)

## 8. Verify & deploy

- [ ] 8.1 Build image with `--features s3-persistence` and push to GHCR
- [ ] 8.2 Update `manifests/homelab.yaml` image tag and deploy to homelab cluster
- [ ] 8.3 Verify restore: delete pod, new pod starts and admin login works immediately
- [ ] 8.4 Verify event flush: create user, delete pod immediately, new pod has user without waiting for cron
