# Mesh Dispatch Locality SLO

## Objective

For a single-node Tachyon deployment, at least 95% of eligible internal mesh
dispatches must use `in_process` over a rolling 15-minute window. The alert
requires at least 100 eligible dispatches and remains pending for 10 minutes
before it fires at warning severity.

Eligible traffic is every `faas_mesh_dispatch_total` sample except
`reason="remote"`. Remote traffic is intentional and cannot satisfy a
single-node locality objective. `saturated` and `pressure` remain in the
denominator because they reveal the local fallbacks the SLO is designed to
detect.

## Enabling the Alert

`manifests/prometheus-mesh-dispatch-slo.yaml` is a Prometheus Operator
`PrometheusRule`. Apply it only after the core-host scrape target is labelled:

```yaml
labels:
  tachyon_mesh_topology: single-node
```

Do not set that label for multi-node deployments. The rule deliberately has no
series for targets without this label, so it cannot misclassify intentional
remote dispatch as a locality regression.

## PromQL

The rule records the eligible count and in-process ratio before evaluating:

```promql
(
  sum by (job, instance, tachyon_mesh_topology) (
    increase(faas_mesh_dispatch_total{tachyon_mesh_topology="single-node",mode="in_process",reason!="remote"}[15m])
  )
  or on (job, instance, tachyon_mesh_topology)
  (0 * tachyon:mesh_dispatch_eligible_total:increase15m)
)
/
clamp_min(tachyon:mesh_dispatch_eligible_total:increase15m, 1)
```

`MeshDispatchLocalityDegraded` fires when the ratio is below `0.95`, the
eligible count is at least `100`, and both conditions persist for `10m`.

## Response

1. Inspect `faas_mesh_dispatch_total` split by `mode` and `reason` for the
   affected `job` and `instance`.
2. If `saturated` dominates, review route concurrency limits and queue depth.
3. If `pressure` dominates, inspect host memory pressure and admission policy.
4. If neither fallback reason explains the ratio, investigate the local mesh
   route resolution before relaxing the threshold.
