# Technical Specification: API Documentation Coverage

## 1. OpenAPI Route Annotations (`core-host/src/host_core/openapi.rs` & Handlers)
Add `#[utoipa::path(...)]` to all unannotated route handlers across the codebase.

**Target Domains:**
- **Guest Lifecycle:** `deploy_function`, `delete_function`, `list_functions`, `function_logs`.
- **State/KV:** `kv_get`, `kv_put`, `kv_delete`, `kv_list_namespaces`.
- **Traffic/Canary:** `set_route_weights`, `get_routing_table`.
- **Resilience:** `run_chaos_scenario`, `get_circuit_breakers`.
- **System:** `worker_status`, `asset_registry_status`.

*Update the central `ApiDoc` struct to include all paths and new component schemas.*
```rust
#[derive(OpenApi)]
#[openapi(
    paths(
        // ... existing 10 routes ...
        crate::mesh::guest_lifecycle::deploy_function,
        crate::mesh::guest_lifecycle::delete_function,
        crate::state::kv::kv_put,
        // ... add the 24 missing paths
    ),
    components(schemas(
        // ... existing schemas ...
        DeployRequest, KvEntry, ChaosScenario, RouteWeight
    ))
)]
pub struct ApiDoc;
```

## 2. Integrity Lock Schema (`core-host/src/host_core/integrity_config.rs`)
Derive `JsonSchema` for the `IntegrityLock` data structures.

```rust
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Debug)]
pub struct IntegrityLock {
    pub version: String,
    pub signatures: std::collections::HashMap<String, String>,
    pub required_capabilities: Vec<String>,
}
```

Expose the endpoint in the admin router:
```rust
// Route: GET /admin/schema/integrity-lock
pub async fn get_integrity_schema() -> impl IntoResponse {
    let schema = schemars::schema_for!(IntegrityLock);
    axum::Json(schema)
}
```