## ADDED Requirements

### Requirement: The admin API surface MUST be conditional on the admin-plane feature
The admin API surface described elsewhere in this capability (manifest schema exposure, OpenAPI/Swagger docs, and the full `/admin/*` route set) SHALL be compiled and mounted only when the `admin-plane` Cargo feature is enabled. The feature SHALL be part of `default`, so this capability's other requirements continue to hold unmodified for default builds; see the `worker-dataplane-profile` capability for the disabled-feature behavior.

#### Scenario: Admin schema endpoints unaffected on a default build
- **WHEN** `core-host` is built with default features
- **THEN** `GET /admin/schema/manifest`, `GET /admin/schema/openapi.json`, and `GET /admin/docs` behave exactly as specified elsewhere in this capability
