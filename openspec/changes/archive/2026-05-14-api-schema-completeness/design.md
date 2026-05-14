# Design: api-schema-completeness

## Tasks 1 & 2 — OpenAPI Coverage Expansion (`openapi.rs`)

`ApiDoc` expanded from 10 paths to **35 paths** across 11 tags:

| Tag | New paths added |
|---|---|
| kv | GET/PUT/DELETE `/admin/kv/{namespace}/{key}` |
| canary | GET/PATCH/POST `/admin/canary` |
| traffic | GET `/admin/shadow/diffs` |
| chaos | POST `/admin/chaos/scenarios` |
| identity | GET `/admin/identity/public-key` |
| identity (enrollment) | POST start/approve, GET poll |
| security | POST recovery-codes, 2fa/regenerate, step-up, pats |
| iam | DELETE user, POST+DELETE group, GET audit logs |
| status | GET kv-cache stats, DELETE kv-cache |
| assets | POST assets, POST models/init, PUT models/upload, POST models/commit |
| schema | GET `/admin/schema/integrity-lock` (new) |

Three new `ToSchema` types: `CanaryStatusEntry`, `ShadowDiff`, `AuditLogEntry`.

Path stub functions remain `#[allow(dead_code)]` since they carry utoipa metadata only.

## Task 3 — `GET /admin/schema/integrity-lock`

A new Axum handler `admin_integrity_lock_schema_handler` is added to `app_runtime.rs`. It returns a handcrafted JSON Schema (Draft-07) describing the `integrity.lock` file structure including:
- Top-level fields (`configVersion`, `hostAddress`, `routes`, …)
- Route entry properties including the new `resourcePolicy.vramMb` and `gpuAffinity` fields added in `feature-parity-vram-kv2`
- Canary config sub-schema

The route `/admin/schema/integrity-lock` is registered in `build_app` under the admin auth middleware, consistent with the other schema endpoints.

## Task 4 — Validation

`cargo check -p core-host` and `cargo clippy --workspace --all-targets` both pass with zero errors.
