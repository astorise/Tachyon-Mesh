## ADDED Requirements

### Requirement: Observability documentation defines mesh dispatch locality policy
The observability documentation SHALL define the single-node mesh dispatch
locality objective, its `tachyon_mesh_topology="single-node"` scope, and the
`MeshDispatchLocalityDegraded` alert semantics alongside the existing metrics.

#### Scenario: Operator inspects locality alert policy
- **WHEN** an operator consults the mesh dispatch observability documentation
- **THEN** it identifies the 95% in-process objective, 15-minute window,
  100-request minimum, and 10-minute alert hold
- **AND** it explains that `reason="remote"` samples are excluded
