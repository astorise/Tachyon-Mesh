# traffic-management-config Delta

## ADDED Requirements

### Requirement: Configuration MUST use a typed WIT contract
The configuration API SHALL validate all routing intents against a strict `config-routing.wit` interface to prevent malformed data from reaching the `core-host` and triggering panics.

#### Scenario: Submitting a valid routing configuration
- **WHEN** the `system-faas-config-api` receives a `TrafficConfiguration` payload
- **THEN** it validates the structure against the WIT `validate-traffic-config` function
- **AND** returns a typed error string if validation fails, without panicking the host.

### Requirement: L4 and L7 routing MUST be decoupled from Gateways
The data model SHALL separate `Gateways` (Ports/Listeners) from `Routes` (L4/L7 matching logic) to allow dynamic attachment of routes to existing ports without restarting the listener.

#### Scenario: Adding a new HTTP route to an existing Gateway
- **GIVEN** an active HTTPS gateway on port 443
- **WHEN** a new HTTP route is deployed referencing that gateway
- **THEN** the router updates its L7 trie/matchers in memory
- **AND** the L4 socket listener remains uninterrupted.
