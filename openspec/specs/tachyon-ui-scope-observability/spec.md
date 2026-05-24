# tachyon-ui-scope-observability Specification

## Purpose
TBD - created by archiving change tachyon-ui-scope-config. Update Purpose after archive.
## Requirements
### Requirement: Observability panel shows per-route scope denial counters
The `TachyonObservabilityPanel` SHALL display a "Scope Denials" widget that shows the lifetime denial count per category for the selected route, sourced from `GET /admin/metrics`.

#### Scenario: Widget displays denial counts when route is selected
- **WHEN** the operator opens the Observability panel with a route selected
- **THEN** the Scope Denials widget shows a table with one row per category that has at least one denial
- **AND** each row shows: category name, count, and a mini spark-bar relative to the highest-count category
- **AND** categories with zero denials are hidden by default

#### Scenario: Widget shows empty state when no denials recorded
- **WHEN** a route has no scope denials (`scopeDenialTotal` is 0)
- **THEN** the widget shows: "No scope denials recorded — scopes are working correctly."

#### Scenario: Widget auto-refreshes every 30 seconds
- **WHEN** the Scope Denials widget is visible
- **THEN** it polls `GET /admin/metrics` every 30 seconds
- **AND** increments in denial counts animate smoothly without full re-render

### Requirement: Route cards in the routing dashboard show an allow-all badge
Each route card in the routing dashboard SHALL display an amber "ALLOW ALL" badge when the route resolves to allow-all scopes, to guide the operator toward phase-2 tightening.

#### Scenario: Route card shows allow-all badge for unscoped route
- **WHEN** a route has no `scopes:` block in its manifest
- **THEN** its card in the routing dashboard shows an amber pill badge labelled "allow-all"
- **AND** hovering the badge shows a tooltip: "This deployment grants all WIT imports. Click to configure scopes."

#### Scenario: Route card shows no badge when explicit scopes are configured
- **WHEN** a route has an explicit `scopes:` block with at least one category
- **THEN** its card shows no scope badge
- **AND** a green "scoped" indicator appears instead

#### Scenario: Clicking the allow-all badge navigates to the Scopes panel
- **WHEN** the operator clicks the amber "allow-all" badge on a route card
- **THEN** the UI navigates to the route detail view with the Scopes panel pre-expanded

### Requirement: Scope Denials widget links to the Scopes panel for quick remediation
Each denial row in the Scope Denials widget SHALL include a "Configure" action that navigates to the Scopes panel with the corresponding category pre-focused.

#### Scenario: Operator clicks Configure on a kv denial row
- **WHEN** the operator clicks "Configure" on the kv row in the Scope Denials widget
- **THEN** the UI navigates to the route detail Scopes panel
- **AND** the kv category row is auto-expanded and focused

