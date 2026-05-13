# core-host Specification

## Purpose
Core Tachyon Mesh host runtime — manifest schema exposure, routing, and admin API surface.

## Requirements

### Schema Generation

## 1. Schema Derivation
Add the `schemars` crate to `core-host/Cargo.toml`.
Derive `JsonSchema` on `Manifest` and all its composite types (`FunctionManifest`, `ResourceLimit`, etc.).

```rust
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Debug)]
pub struct Manifest {
    pub api_version: String,
    pub kind: String,
    pub metadata: Metadata,
    pub spec: ManifestSpec,
}
```

## 2. Schema Endpoint
Create an unauthenticated (or read-only admin) route `GET /admin/schema/manifest` that returns the generated schema.

```rust
// In router definition
let schema = schemars::schema_for!(Manifest);
serde_json::to_string(&schema)
```

## 3. Dry-Run Routing
Ensure that requests to `POST /admin/manifest/dryrun` are actively routed to the `system-faas-config-api` component rather than being evaluated natively by the host.

## Requirements: OpenAPI Documentation Endpoints

### Requirement: core-host MUST expose an utoipa-generated OpenAPI 3.1 schema
The core-host admin API SHALL expose `GET /admin/schema/openapi.json` returning an OpenAPI 3.1 JSON document generated at compile time via the `utoipa` crate. An `ApiDoc` struct SHALL annotate the top 10 most critical admin routes and key response types.

#### Scenario: Schema endpoint returns valid OpenAPI JSON
- **WHEN** an authenticated client sends `GET /admin/schema/openapi.json`
- **THEN** the response status is 200 with `content-type: application/json`
- **AND** the body is a valid OpenAPI 3.1 document containing paths for `/admin/manifest`, `/admin/iam/users`, and `/admin/metrics`

### Requirement: core-host MUST serve an interactive API documentation page
The core-host admin API SHALL expose `GET /admin/docs` returning a Swagger UI HTML page embedded at compile time via `include_str!`. No filesystem access shall occur at runtime to serve this page.

#### Scenario: Docs page is self-contained
- **WHEN** an authenticated client sends `GET /admin/docs`
- **THEN** the response status is 200 with `content-type: text/html; charset=utf-8`
- **AND** the HTML references `/admin/schema/openapi.json` as the schema URL

### Requirement: Cross-layer validation MUST assert OpenAPI contract routes exist
The `validate_cross_layer.sh` script SHALL assert that the four core OpenAPI contract routes (`/admin/schema/openapi.json`, `/admin/docs`, `/admin/manifest`, `/admin/iam/users`) are registered in `app_runtime.rs`.

#### Scenario: Validation fails when a contract route is removed
- **WHEN** one of the checked routes is removed from the Axum router
- **THEN** `validate_cross_layer.sh` exits with a non-zero status and names the missing route