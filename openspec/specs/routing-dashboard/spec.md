# routing-dashboard Specification

## Purpose
Document the retirement of the legacy `<tachyon-routing-dashboard>` form. The
active routing surface is `<tachyon-routing-panel>`, which reads live route
state and writes real `IntegrityConfig` fields.

## Requirements
### Requirement: Legacy routing dashboard is retired
The Tachyon UI SHALL NOT register or route `<tachyon-routing-dashboard>`.
Routing changes that affect runtime behavior SHALL be implemented in
`<tachyon-routing-panel>` through manifest-backed controllers.

#### Scenario: Routing navigation resolves to manifest-backed panel
- **GIVEN** the App Shell is visible
- **WHEN** the operator selects the `routing` navigation item
- **THEN** the router outlet displays `<tachyon-routing-panel>`
- **AND** it does not display `<tachyon-routing-dashboard>`

#### Scenario: Legacy TrafficConfiguration payload is not submitted
- **WHEN** the operator edits routing controls
- **THEN** the frontend applies changes through `apply_manifest_config`
- **AND** it does not submit a `TrafficConfiguration` payload through a legacy domain command
