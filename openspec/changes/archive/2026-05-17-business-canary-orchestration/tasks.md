# Implementation Tasks

- [ ] **Task 1: WIT Definition & Bindings**
  - Create `wit/telemetry/custom-metrics.wit`.
  - Run `wasmtime-wit-bindgen` to generate the Rust structures.

- [ ] **Task 2: Prometheus Integration**
  - In `core-host/src/telemetry.rs` (or equivalent), implement the `push_custom_metric` logic.
  - Ensure dynamic metric creation uses `prometheus::default_registry()` so the values are automatically exposed on the existing `/metrics` endpoint.

- [ ] **Task 3: Canary Engine Updates**
  - Update `system-faas-gitops-broker` (the component managing the Canary loop).
  - Ensure the evaluation phase executes a generic PromQL query against the local metrics endpoint for *every* metric defined in the manifest, without hardcoding logic exclusively for L4/L7 errors.

- [ ] **Task 4: Schema Alignment**
  - Update `system-faas-config-api/src/schemas/manifest.json` (and `integrity.lock` if applicable) to officially support the custom metric namespace definition in the YAML deployment configurations.
