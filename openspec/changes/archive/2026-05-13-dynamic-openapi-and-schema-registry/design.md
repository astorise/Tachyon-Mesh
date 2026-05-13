# Design: dynamic-openapi-and-schema-registry

## Overview

Adds a compile-time OpenAPI 3.1 schema derived from utoipa annotations on the core-host admin routes, and exposes it via two new endpoints alongside an interactive Swagger UI. A new `system-faas-openapi` WASM component bundles the Swagger UI HTML.

## Architecture

### Compile-Time Schema (core-host)

`core-host/src/host_core/openapi.rs` declares an `ApiDoc` struct using `#[derive(utoipa::OpenApi)]`. It annotates the top 10 critical admin routes as shadow path-declaration functions (which never run, they only carry metadata) and derives `#[utoipa::ToSchema]` on the key response types.

The module exposes `get_base_openapi_schema() -> String` which calls `ApiDoc::openapi().to_pretty_json()`. This is the single source of truth for the machine-readable API contract.

**Annotated routes (top 10):**
1. `GET /admin/status`
2. `GET /admin/metrics`
3. `GET /admin/manifest`
4. `POST /admin/manifest`
5. `POST /admin/manifest/bundle`
6. `GET /admin/schema/manifest`
7. `GET /admin/schema/openapi.json` (new)
8. `GET /admin/iam/users`
9. `PATCH /admin/iam/users/{username}`
10. `GET /admin/iam/groups`

### New Admin Endpoints (core-host)

Two new Axum handlers added to `build_app()` under the existing admin auth middleware:

- `GET /admin/schema/openapi.json` → `admin_openapi_schema_handler` — calls `get_base_openapi_schema()` and returns JSON with `content-type: application/json`.
- `GET /admin/docs` → `admin_openapi_docs_handler` — serves `swagger-ui.html` (embedded via `include_str!`) with `content-type: text/html`.

The Swagger UI HTML is also embedded directly in `core-host/src/host_core/swagger-ui.html` so the binary is self-contained (no runtime file I/O for docs).

### system-faas-openapi WASM Component

`systems/system-faas-openapi` is a `wasm32-wasip2` cdylib component using the `system-faas-guest` WIT world. It:
- Implements `handler::handle-request`
- Serves `GET /admin/docs` → embedded Swagger UI HTML (`include_str!("swagger-ui.html")`)
- Proxies `GET /admin/schema/openapi.json` → calls `outbound_http::send_request` to the host loopback, allowing the schema to be served through the WASM component layer when routed there
- Returns 404 for all other paths

The Swagger UI HTML inside the WASM component is identical to the one in core-host, ensuring zero drift between the two serving paths.

### Build Script

`scripts/build-guest-artifacts.sh` now includes:
```bash
cargo build -p system-faas-openapi --target wasm32-wasip2 --release
```

### Cross-Layer Validation

`scripts/validate_cross_layer.sh` now asserts that `/admin/schema/openapi.json`, `/admin/docs`, `/admin/manifest`, and `/admin/iam/users` routes exist in `app_runtime.rs`, ensuring the OpenAPI contract cannot silently diverge from the router.

## Pre-existing Bug Fixes

While implementing this change, four pre-existing compilation errors were repaired:
1. `IntegrityRoute` derived `Eq` but `CanaryConfig` (with `f32` field) doesn't implement `Eq` → removed `Eq` from `IntegrityRoute`.
2. Missing `canary` field initialization in `integrity_config.rs:normalize_route()`.
3. `HostTable` trait methods (`get`, `set`, `delete`, `batch_set`, `get_range`, `new`) had incorrect return types for wasmtime 44's bindgen (which no longer wraps WIT `result<...>` in `wasmtime::Result`).
4. `json!` macro and `spawn_canary_evaluators` were not re-exported via `host_core.rs`; `#![recursion_limit = "256"]` was missing for the large `json!` literal.
