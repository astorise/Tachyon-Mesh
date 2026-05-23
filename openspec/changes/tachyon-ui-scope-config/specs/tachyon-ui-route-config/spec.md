## ADDED Requirements

### Requirement: Route detail view includes a Scopes panel alongside Volumes and Concurrency panels
The route detail view SHALL render a Scopes panel (implemented by `tachyon-ui-scope-editor`) as a peer panel alongside the existing Volumes and Concurrency Policy panels.

#### Scenario: Scopes panel appears in the route detail tab layout
- **WHEN** a user navigates to the detail view of any FaaS route
- **THEN** a "Scopes" tab or section is visible in the panel layout alongside "Volumes" and "Concurrency"
- **AND** the Scopes panel is pre-expanded when the route resolves to allow-all (to prompt configuration)

#### Scenario: Scopes panel is collapsed by default for explicitly scoped routes
- **WHEN** the route has an explicit non-allow-all `scopes:` block
- **THEN** the Scopes panel is collapsed by default
- **AND** its header shows a summary chip count (e.g. "3 categories, 7 patterns")
