All tasks below were completed in PR #336 (branch `codex/issue-310-worker-dataplane-profile`), closing issue #310.

## 1. Cargo feature and route gating

- [x] 1.1 Add `admin-plane` feature to `core-host/Cargo.toml`, included in `default`
- [x] 1.2 Gate `mod admin_plane;` and `mod openapi;` (plus their re-exports) behind `#[cfg(feature = "admin-plane")]` in `core-host/src/host_core.rs`
- [x] 1.3 Move `bootstrap_routes()` (`/admin/enrollment/start`, `/admin/enrollment/poll/{session_id}`) from `admin_plane.rs` into `integrity_config.rs` so it stays unconditionally compiled
- [x] 1.4 Update `build_app()` in `app_runtime.rs` to conditionally merge `admin_plane::authenticated_routes(...)` and unconditionally merge `bootstrap_routes()`

## 2. Dead-code gating for orphaned items

- [x] 2.1 Run `RUSTFLAGS="-D dead_code" cargo check -p core-host --no-default-features --features ring` to find every item that becomes unreachable without `admin-plane`
- [x] 2.2 Annotate orphaned items in `auth.rs` (IAM/security handlers), `system_storage.rs` (upload handlers), `kv_cache.rs` (admin handlers), `volume_backup.rs` (restore/list), `mesh_dispatch_metrics.rs`, `scoping/mod.rs`, `memory_governor.rs`, and `domain_types.rs` with `#[cfg_attr(not(feature = "admin-plane"), allow(dead_code))]` (or hard `#[cfg]` where nothing else, including tests, calls them)
- [x] 2.3 Annotate the manifest/nodes/bundle cluster in `integrity_config.rs` (~28 items) the same way
- [x] 2.4 Fix the resulting unused-import warning (`delete`/`patch`/`put` in `host_core.rs`) by scoping that import behind `admin-plane`
- [x] 2.5 Confirm `cargo check` is clean (zero warnings) for both default and worker-profile feature sets

## 3. Tests

- [x] 3.1 Gate router-level tests that assert on the full `/admin/*` surface (`telemetry_and_l4.rs`, `http_router.rs::other_admin_routes_still_require_auth`, `iam_management.rs`'s unauthenticated-caller tests) behind `#[cfg(feature = "admin-plane")]`
- [x] 3.2 Add `worker_profile_completes_enrollment_bootstrap_with_no_admin_surface` in `http_router.rs`, gated `#[cfg(not(feature = "admin-plane"))]`, asserting enrollment start/poll succeed and `/admin/nodes` 404s
- [x] 3.3 Run `cargo clippy -p core-host --all-targets -- -D warnings -D clippy::unwrap_used` for both feature sets
- [x] 3.4 Run `cargo nextest run` targeted at the affected test modules for both feature sets against real WASM guest artifacts and confirm pass

## 4. CI and release surface

- [x] 4.1 Add a worker-profile entry to `feature-matrix-tests` and `feature-matrix-artifacts` in `.github/workflows/ci.yml`
- [x] 4.2 Add the corresponding docker-artifact label case
- [x] 4.3 Comment the pre-existing bare `--no-default-features` entry noting it now also drops `admin-plane`

## 5. Documentation

- [x] 5.1 Add "Path C — Worker / Data-Plane Node" to `README.md`
- [x] 5.2 Add an admin-plane caveat to `docs/ide-integration.md`
- [x] 5.3 Add a `CHANGELOG.md` entry
- [x] 5.4 Write this retroactive OpenSpec proposal/design/specs/tasks set and archive it into `openspec/specs/`
