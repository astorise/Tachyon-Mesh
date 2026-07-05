## Why

`admin_plane.rs` was extracted from the core-host router in #295 as groundwork toward a "minimalist core." That extraction made it possible to compile a worker binary with no admin HTTP surface at all — smaller attack surface (no `/admin/*` endpoints to probe on a compute node), a smaller binary, and faster startup — while still letting the worker receive its configuration over the existing gossip/config-update path and remain enrollable. Issue #310 requests exactly this: a Cargo feature that gates the admin plane so operators can opt a node out of it at build time.

## What Changes

- Add a new core-host Cargo feature `admin-plane`, part of `default`, gating the entire `/admin/*` authenticated HTTP surface: `core-host/src/host_core/admin_plane.rs` (IAM, manifest/canary/chaos control, node registry, asset uploads) and `core-host/src/host_core/openapi.rs` (OpenAPI/Swagger docs) are now `#[cfg(feature = "admin-plane")]` modules, and `build_app()` only merges `admin_plane::authenticated_routes(...)` when the feature is on.
- Move the enrollment-bootstrap routes (`POST /admin/enrollment/start`, `GET /admin/enrollment/poll/{session_id}`) out of `admin_plane.rs` into `core-host/src/host_core/integrity_config.rs`, which stays compiled regardless of `admin-plane` — a worker node must remain enrollable, and must stay a valid answering peer when another unenrolled node's outbound enrollment call lands on it. `POST /admin/enrollment/approve` (operator PIN approval) stays behind `admin-plane`, since approval is expected to happen against an admin-plane node via Tachyon Studio.
- Annotate every function/type/field elsewhere in the crate that becomes unreachable-but-still-compiled without `admin-plane` (in `auth.rs`, `system_storage.rs`, `kv_cache.rs`, `volume_backup.rs`, `mesh_dispatch_metrics.rs`, `scoping/mod.rs`, `memory_governor.rs`, `domain_types.rs`, and a manifest/nodes/bundle cluster in `integrity_config.rs`) with `#[cfg_attr(not(feature = "admin-plane"), allow(dead_code))]` rather than removing it, so both feature configurations keep compiling clean under the CI feature-matrix's `RUSTFLAGS="-D dead_code"`.
- Add a worker-profile entry (`--no-default-features --features ring,rate-limit,resiliency,mtls,secrets-vault,websockets`) to the `feature-matrix-tests` and `feature-matrix-artifacts` jobs in `.github/workflows/ci.yml`, plus a corresponding docker-image artifact label.
- Add an integration test, `worker_profile_completes_enrollment_bootstrap_with_no_admin_surface` (`core-host/src/host_core/tests/http_router.rs`), that drives the real router end to end: enrollment start/poll still succeed, `/admin/nodes` returns 404 (not 401) — proving the surface is absent, not merely unauthenticated.
- Document the profile: a new "Path C — Worker / Data-Plane Node" section in `README.md`, a caveat in `docs/ide-integration.md` that its schema endpoints require an admin-plane node, and a `CHANGELOG.md` entry.
- **Not done** (explicitly out of scope): a pre-built worker release artifact via `get-tachyon.sh`/`get-tachyon.ps1`, and a worker Docker image variant. Neither the release scripts nor the Dockerfiles support `--no-default-features` today (`CARGO_FEATURES` is additive-only); shipping either would need release-pipeline changes and is left as a future follow-up.

## Capabilities

### New Capabilities
- `worker-dataplane-profile`: the `admin-plane` Cargo feature, what it gates, and the guarantee that enrollment bootstrap survives with it disabled.

### Modified Capabilities
- `core-host`: the admin API surface described here is now conditional on the `admin-plane` Cargo feature (on by default) rather than always compiled in.
- `github-actions`: the feature-matrix jobs gain a sixth combination (the worker profile) alongside the existing five.
- `zero-touch-enrollment`: the "any ready peer can serve `/admin/enrollment/start`" bootstrap-discovery guarantee now explicitly covers worker-profile peers built with `admin-plane` disabled.

## Impact

- Affected code: `core-host/Cargo.toml`; `core-host/src/host_core/{admin_plane,app_runtime,integrity_config}.rs`; `core-host/src/{auth,system_storage,memory_governor}.rs`; `core-host/src/host_core/{kv_cache,volume_backup,mesh_dispatch_metrics,domain_types}.rs`; `core-host/src/host_core/scoping/mod.rs`; `core-host/src/host_core/tests/{http_router,iam_management,telemetry_and_l4}.rs`.
- Affected CI: `.github/workflows/ci.yml` (`feature-matrix-tests`, `feature-matrix-artifacts`).
- Affected docs: `README.md`, `docs/ide-integration.md`, `CHANGELOG.md`.
- No breaking change to the default build: `admin-plane` ships in `default`, so existing deployments and the bare `--no-default-features` CI/release combinations that didn't previously care about the admin surface now also build without it — call out in the feature-matrix job comments since it changes what that pre-existing combination validates.
- Delivered in PR #336 (branch `codex/issue-310-worker-dataplane-profile`), closing issue #310.
