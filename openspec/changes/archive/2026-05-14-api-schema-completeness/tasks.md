# Implementation Tasks

- [x] **Task 1: Guest Lifecycle & KV OpenAPI**
  - Add `#[utoipa::path]` macros to all FaaS deployment and logging endpoints.
  - Add macros to all Key-Value storage endpoints.
  - Register the corresponding data structures in `ApiDoc`.

- [x] **Task 2: Traffic & Chaos OpenAPI**
  - Add macros to routing, canary, circuit breaker, and chaos testing endpoints.
  - Register the corresponding data structures in `ApiDoc`.

- [x] **Task 3: Integrity Schema Generation**
  - Add `schemars::JsonSchema` derivation to `IntegrityLock` and its nested structs.
  - Create the `GET /admin/schema/integrity-lock` HTTP handler.
  - Wire the handler into the main `axum` router.

- [x] **Task 4: Validation**
  - Compile `core-host`.
  - Navigate to `http://localhost:8080/admin/docs` and verify that the Swagger UI now displays the complete list of ~34 operations.
  - Fetch `http://localhost:8080/admin/schema/integrity-lock` to verify the JSON schema is valid.
