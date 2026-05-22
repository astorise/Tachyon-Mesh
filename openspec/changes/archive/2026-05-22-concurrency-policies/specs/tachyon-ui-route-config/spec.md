## ADDED Requirements

### Requirement: Route detail view exposes a Concurrency Policy panel with risk badges
The Tachyon-UI SHALL provide a Concurrency Policy panel in the route detail view that lets the operator configure the `concurrency`, `consistency`, and `coordination` modes, displaying a risk-level badge and a tooltip with a concrete failure scenario for each selection.

#### Scenario: Selecting a high-risk combination surfaces a red badge with tooltip
- **WHEN** an operator selects `concurrency.mode: "unrestricted"` and a shared volume with `consistency.write_mode: "last_write_wins"`
- **THEN** the panel displays a red `High Risk` badge next to the volume row
- **AND** hovering the badge shows a tooltip: "Concurrent invocations will silently overwrite each other's writes."

#### Scenario: Selecting a low-risk combination surfaces a green badge
- **WHEN** an operator selects `concurrency.mode: "mesh-singleton"` and `consistency.write_mode: "pessimistic_lock"`
- **THEN** the panel displays a green `Low Risk` badge
- **AND** the tooltip explains the latency trade-off: "All invocations serialize through a distributed lock; expect added latency under load."

#### Scenario: Panel hides incompatible combinations
- **WHEN** the operator selects `consistency.write_mode: "pessimistic_lock"`
- **AND** the route's `concurrency.mode` is `"unrestricted"`
- **THEN** the panel surfaces an inline warning that pessimistic locking only makes sense with singleton modes
- **AND** offers a one-click fix to switch concurrency to `mesh-singleton`

#### Scenario: Panel exposes data attributes for future simulation hook
- **WHEN** the panel renders any mode option
- **THEN** the option element carries a `data-sim-scenario="<scenario-id>"` attribute
- **AND** a future JS simulation script can attach to those attributes without modifying the panel
