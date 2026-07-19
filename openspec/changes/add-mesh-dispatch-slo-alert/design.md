## Context

Issue #307 already emits `faas_mesh_dispatch_total{mode,reason}` and
`faas_mesh_dispatch_duration_seconds{mode}`. The metrics do not, however,
state what percentage of local traffic must stay in-process or tell an
operator when that property regresses. The repository has raw Kubernetes
manifests but no shared Prometheus Operator configuration, so the alert must
be self-contained and explicitly opt in to the topology where it is valid.

## Goals / Non-Goals

**Goals:**
- Define a measurable 95% in-process locality objective over 15 minutes.
- Ship a reusable PrometheusRule and an operator runbook.
- Avoid false alerts from low request volume and multi-node deployments.

**Non-Goals:**
- Change dispatch routing or add labels to the request path.
- Infer cluster topology at runtime.
- Enforce a locality objective for multi-node deployments.

## Decisions

### Fixed locality objective with a traffic floor

The objective is at least 95% `in_process` dispatches among eligible internal
dispatches in a rolling 15-minute window. The alert only evaluates windows
with 100 or more eligible dispatches and must remain breached for 10 minutes.
Eligible dispatches exclude `reason="remote"`; saturation and pressure
fallbacks remain in the denominator.

### Topology is an explicit scrape label

The host exports no authoritative single-node topology metric. The rule
therefore selects only targets labelled `tachyon_mesh_topology="single-node"`
by Prometheus scrape configuration, keeping multi-node traffic out of the
locality objective without changing application metrics.

### PrometheusRule plus a versioned runbook

The repository ships an optional Prometheus Operator `PrometheusRule` and a
runbook. Operations can enable, tune, or roll back policy without changing the
request path.

## Risks / Trade-offs

- [A scrape target lacks the topology label] -> The rule emits no series; the
  runbook makes the required label explicit.
- [A low-throughput deployment regresses] -> The 100-request floor suppresses
  noise; operators can inspect the existing counters or tune a local rule.
- [The threshold is too strict for a workload] -> The optional baseline rule
  documents the adjustment point.

## Migration Plan

1. Add `tachyon_mesh_topology="single-node"` to the core-host scrape target.
2. Apply the PrometheusRule and observe the 15-minute recording rules.
3. Remove the PrometheusRule to roll back alerting; no host restart is needed.

## Open Questions

None. The baseline is 95% over 15 minutes, 100 eligible requests, and a
10-minute alert hold.
