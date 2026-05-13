# Technical Specification: Compile-Time OpenAPI Base

## 1. Utoipa Integration
Add `utoipa` to `core-host/Cargo.toml`. Decorate existing route handlers and domain structs.

```rust
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        crate::auth::login,
        crate::mesh::get_topology,
        // ... other routes
    ),
    components(schemas(Manifest, DryRunResult, ValidationError))
)]
pub struct ApiDoc;
```

## 2. Host Export
Instead of serving this via `axum` directly, expose it to the WASM environment (or via an internal unexposed memory pipe) so the system FaaS can retrieve it.

```rust
pub fn get_base_openapi_schema() -> String {
    ApiDoc::openapi().to_pretty_json().unwrap()
}
```

## 3. Route Delegation
Configure the L7 router in `core-host` to blindly delegate `GET /admin/docs/*` and `GET /admin/schema/openapi.json` to the new `system-faas-openapi` component.