# logical-plane Specification Delta

## ADDED Requirements

### Requirement: View Builder Component
The mesh SHALL provide a `system-faas-view-builder` component that consumes CDC mutation events and produces materialized view payloads under the `V:` keyspace.

#### Scenario: CDC event rebuilds a view
- **WHEN** the view builder receives a mutation event
- **THEN** it emits a materialized view payload keyed by `V:<event-key>`

### Requirement: CDC Ordering For Views
The view builder SHALL inspect vector clocks on mutation events and reject stale events whose vector clock is older than the currently materialized view.

#### Scenario: Stale event arrives
- **WHEN** a mutation event has a lower vector clock than the current view
- **THEN** the view builder ignores the event instead of overwriting the view

### Requirement: Gateway Materialized View Fast Path
The system gateway SHALL map `/api/views/{view_name}` requests to the `V:{view_name}` materialized view keyspace without invoking SQL or graph engines.

#### Scenario: View route is requested
- **WHEN** a client requests `/api/views/dashboard/user123`
- **THEN** the gateway resolves the fast-path key `V:dashboard/user123`

### Requirement: View Builders Run At Background Priority
View builder work SHALL be marked as background priority so materialization does not stall synchronous user-facing FaaS requests.

#### Scenario: View builder emits work metadata
- **WHEN** a view rebuild is accepted
- **THEN** the component response identifies the work as background priority
