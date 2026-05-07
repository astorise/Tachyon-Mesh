# routing-dashboard Specification

## Purpose
TBD - created by archiving change tachyon-ui-webcomponents-phase3. Update Purpose after archive.
## Requirements
### Requirement: Tachyon UI MUST expose routing configuration as a Web Component
The Tachyon UI SHALL render L4 gateway and L7 route configuration through a native `<tachyon-routing-dashboard>` custom element mounted inside the App Shell router outlet.

#### Scenario: Routing dashboard is reachable from the App Shell
- **GIVEN** the App Shell is visible
- **WHEN** the operator selects the `routing` navigation item
- **THEN** the router outlet displays `<tachyon-routing-dashboard>`.

### Requirement: Routing Dashboard MUST submit strict routing payloads
The Routing Dashboard SHALL construct a `TrafficConfiguration` payload matching the Rust Serde validator and submit it to `apply_configuration` with the `config-routing` domain.

#### Scenario: Valid routing form applies configuration
- **GIVEN** the operator enters a gateway, path prefix, and target workload
- **WHEN** the routing form is submitted
- **THEN** the component calls `apply_configuration`
- **AND** successful validation emits `config:applied`.

#### Scenario: Invalid routing payload is handled in the component
- **GIVEN** the backend rejects the routing payload
- **WHEN** `apply_configuration` returns a failed response or throws
- **THEN** the component displays the failure in its feedback zone
- **AND** it emits `config:error`.

