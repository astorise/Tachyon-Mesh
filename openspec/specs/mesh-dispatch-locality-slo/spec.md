# mesh-dispatch-locality-slo Specification

## Purpose
TBD - created by archiving change add-mesh-dispatch-slo-alert. Update Purpose after archive.
## Requirements
### Requirement: Single-node mesh dispatch locality SLO
The mesh SHALL maintain at least 95% `in_process` dispatches among eligible
internal dispatches over a rolling 15-minute window for Prometheus targets
labelled `tachyon_mesh_topology="single-node"`. Eligible dispatches SHALL
include all `faas_mesh_dispatch_total` modes except samples whose `reason` is
`remote`.

#### Scenario: Healthy single-node dispatch mix
- **WHEN** at least 100 eligible dispatches occur in 15 minutes and 95% or more use `mode="in_process"`
- **THEN** the locality objective is met
- **AND** no locality degradation alert is active

#### Scenario: Remote dispatches do not dilute the locality ratio
- **WHEN** a single-node target records dispatches with `reason="remote"`
- **THEN** those samples are excluded from the locality SLO denominator
- **AND** saturation and pressure fallbacks remain in the denominator

### Requirement: Sustained locality degradation alerts operators
The deployment SHALL provide an optional `MeshDispatchLocalityDegraded`
Prometheus alert for labelled single-node targets. It SHALL fire only when at
least 100 eligible dispatches were observed in 15 minutes and the in-process
ratio remains below 95% for 10 minutes.

#### Scenario: Sustained fallback triggers the alert
- **WHEN** the in-process ratio is below 95% for a labelled single-node target
- **AND** the 15-minute eligible dispatch count is at least 100
- **AND** the breach persists for 10 minutes
- **THEN** `MeshDispatchLocalityDegraded` fires with a warning severity

#### Scenario: Sparse traffic does not trigger the alert
- **WHEN** fewer than 100 eligible dispatches occur in the 15-minute window
- **THEN** `MeshDispatchLocalityDegraded` does not fire

