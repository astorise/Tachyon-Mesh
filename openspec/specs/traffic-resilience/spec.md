# traffic-resilience Specification

## Purpose
TBD - created by archiving change tachyon-ui-traffic-dashboards. Update Purpose after archive.
## Requirements
### Requirement: Tachyon UI MUST expose a routing panel Web Component
Tachyon UI SHALL provide a `<tachyon-routing-panel>` Web Component for previewing sealed routes and editing manifest-backed routing controls.

#### Scenario: Routing panel renders manifest controls
- **GIVEN** the App Shell mounts `<tachyon-routing-panel>`
- **WHEN** the component renders
- **THEN** it displays sealed route previews from `get_mesh_graph`
- **AND** it displays manifest-backed controls for layer4 bindings, TEE backend, telemetry sample rate, instance pool memory, cloud sync endpoint, batch targets, and scope enforcement
- **AND** it includes a feedback zone.

#### Scenario: Routing panel applies through manifest controller
- **GIVEN** the operator edits routing manifest controls
- **WHEN** the form is submitted
- **THEN** the panel mutates the corresponding `IntegrityConfig` fields
- **AND** applies the updated manifest through `apply_manifest_config`
- **AND** it does not submit a legacy `config-routing` payload.

### Requirement: Tachyon UI MUST expose a resilience panel Web Component
Tachyon UI SHALL provide a `<tachyon-resilience-panel>` Web Component for configuring route-level timeout and retry values.

#### Scenario: Resilience panel renders policy controls
- **GIVEN** the App Shell mounts `<tachyon-resilience-panel>`
- **WHEN** the component renders
- **THEN** it displays a route selector, timeout, retry count, and retry status controls
- **AND** it includes an apply action and feedback zone.

#### Scenario: Resilience panel applies through manifest controller
- **GIVEN** the operator enters route resilience policy values
- **WHEN** the apply action is submitted
- **THEN** the panel mutates `routes[].resiliency` on the selected route
- **AND** applies the updated manifest through `apply_manifest_config`
- **AND** validation errors are displayed without unmounting the App Shell.

### Requirement: Traffic dashboards MUST animate successful feedback
Traffic and resilience dashboard feedback SHALL pulse when a configuration is successfully accepted.

#### Scenario: Successful configuration pulses feedback
- **GIVEN** a traffic dashboard receives a successful configuration response
- **WHEN** it renders success feedback
- **THEN** `#feedback-zone` receives a GSAP success pulse
- **AND** the dashboard remains interactive after the animation.
