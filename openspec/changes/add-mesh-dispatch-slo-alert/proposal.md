## Why

The mesh dispatch metrics delivered for #307 show whether internal traffic used
the in-process path, but they do not define the expected healthy ratio or
produce an operator-visible signal when that ratio regresses. A single-node
deployment can silently fall back to UDS or TCP without violating a concrete
service objective.

## What Changes

- Define a single-node mesh dispatch SLO: at least 95% of eligible internal
  dispatches use `in_process` over a rolling 15-minute window.
- Add a Prometheus alert that fires only after the ratio remains below the
  objective for 10 minutes and only when at least 100 eligible dispatches were
  observed in the window.
- Require the alert to be scoped by the externally supplied
  `tachyon_mesh_topology="single-node"` target label, so multi-node traffic is
  not evaluated against a locality objective it cannot satisfy.
- Document the PromQL expression, its exclusions, and its operator response.

## Capabilities

### New Capabilities

- `mesh-dispatch-locality-slo`: Defines the single-node in-process dispatch
  objective and its Prometheus alerting contract.

### Modified Capabilities

- `compute-observability`: The Observability documentation includes the active
  dispatch locality SLO and alert semantics alongside the existing metrics.

## Impact

- Adds a Prometheus Operator `PrometheusRule` manifest and an operator runbook.
- Extends OpenSpec observability requirements without changing the request path
  or the existing metric names and labels.
