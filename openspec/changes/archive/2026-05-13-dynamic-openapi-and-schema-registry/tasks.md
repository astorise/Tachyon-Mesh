# Implementation Tasks

- [x] **Task 1: Core-Host Utoipa Implementation**
  - Add `utoipa` to `core-host/Cargo.toml`.
  - Create the `ApiDoc` struct and annotate the top 10 most critical routes (IAM, Manifests, Topology).
  - Implement the internal accessor `get_base_openapi_schema`.

- [x] **Task 2: Create `system-faas-openapi`**
  - Initialize the new `wasm32-wasip2` project.
  - Implement the routing logic for `/admin/docs` and `/admin/schema/openapi.json`.
  - Embed a lightweight Swagger UI or Redoc HTML template.

- [x] **Task 3: Host Routing**
  - Update `core-host` router to forward `/admin/docs` and schema requests to the new system FaaS.
  - Add the new FaaS to `scripts/build-guest-artifacts.sh`.

- [x] **Task 4: Client Verification**
  - Add a test in `tachyon-client` (or update `validate_cross_layer.sh`) that fetches `openapi.json` from a running cluster and asserts that hardcoded paths (e.g., `/admin/manifest/dryrun`) still exist in the spec.
