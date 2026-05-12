# Technical Specification: Core-Host Schema Generation

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