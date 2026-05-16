# core-host Specification

## Purpose
Core Tachyon Mesh host runtime — manifest schema exposure, routing, and admin API surface.

## Requirements

### Requirement: Manifest schema generation MUST be supported
The core-host SHALL add the `schemars` crate to `core-host/Cargo.toml`, derive `JsonSchema` on `Manifest` and all its composite types (`FunctionManifest`, `ResourceLimit`, etc.), expose `GET /admin/schema/manifest`, and actively route `POST /admin/manifest/dryrun` to the `system-faas-config-api` component rather than evaluating dry runs natively by the host.

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

```rust
// In router definition
let schema = schemars::schema_for!(Manifest);
serde_json::to_string(&schema)
```

#### Scenario: Manifest schema and dry-run routes are available
- **WHEN** an operator requests `GET /admin/schema/manifest`
- **THEN** core-host returns the generated manifest JSON Schema
- **AND** requests to `POST /admin/manifest/dryrun` are routed to `system-faas-config-api`

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
### Requirement: OpenAPI schema MUST cover all ~35 admin routes
The `ApiDoc` struct SHALL declare `#[utoipa::path]` stubs for all admin routes including KV-Partition V2, canary management, shadow diffs, chaos scenarios, enrollment, security (MFA/PAT/step-up), full IAM CRUD, KV-cache, and asset/model upload. At least 35 operations SHALL appear in the generated OpenAPI document.

#### Scenario: OpenAPI schema includes broad admin coverage
- **WHEN** the generated OpenAPI document is inspected
- **THEN** it contains at least 35 operations covering the core admin API surface

### Requirement: `GET /admin/schema/integrity-lock` MUST return a JSON Schema for the lock file
A new endpoint `GET /admin/schema/integrity-lock` SHALL return a JSON Schema (Draft-07) document describing the `integrity.lock` file format including route entries, `resourcePolicy` (with `vramMb`, `gpuAffinity`), and canary config sub-schemas.

#### Scenario: Agent fetches integrity lock schema
- **WHEN** an agent calls `GET /admin/schema/integrity-lock`
- **THEN** the response is JSON with `$schema`, `title: "IntegrityLock"`, and a `routes` array property

### Requirement: core-host MUST expose a zero-copy layer-wise inference WIT contract
The project SHALL define `wit/ai/inference.wit` in the existing `tachyon:mesh@1.1.0` WIT package and SHALL expose a `layer-execution` interface with opaque `tensor-handle` values so Wasm guests can sequence model layers without copying intermediate tensors through linear memory.

#### Scenario: Guest orchestrates layer-wise execution through tensor handles
- **WHEN** a guest calls `load-layer`, `forward-layer`, and `drop-tensor` through the `layer-execution` interface
- **THEN** the host owns tensor memory natively and the guest only receives opaque `tensor-handle` identifiers

### Requirement: AI inference dependencies MUST remain feature-gated
The `core-host` crate SHALL keep heavyweight AI dependencies behind the `ai-inference` feature and SHALL return a clear fallback error for AI guests when the feature is not compiled.

#### Scenario: AI guest runs without ai-inference feature
- **WHEN** `core-host` is built without `--features ai-inference`
- **AND** an AI guest such as `guest-ai` is selected for execution
- **THEN** execution fails gracefully with an error naming the missing `ai-inference` feature
