# FaaS Observability

## Prometheus metrics

All metrics are exported by the host's prometheus endpoint.

### Scope denial metrics

| Metric | Labels | Description |
|--------|--------|-------------|
| `faas_scope_denials_total` | `deployment`, `category` | Runtime WIT import denials, counted per deployment and per category (`secrets`, `kv`, `vector`, etc.). |
| `faas_scopes_allow_all_total` | `deployment` | Invocations where the deployment resolved to `allow-all` scopes (no explicit `scopes:` block in the manifest). Use this to identify deployments that need tightening. |
| `faas_linker_cache_hit_total` | — | Linker cache hits (scope shape already compiled). |
| `faas_linker_cache_miss_total` | — | Linker cache misses (linker built for new scope shape). |

### Admin API

`GET /admin/metrics` returns a JSON summary including:

```json
{
  "scopeDenialTotal": 42,
  ...
}
```

`scopeDenialTotal` is the lifetime count of runtime scope denials across all deployments and categories. For per-deployment, per-category breakdowns use `faas_scope_denials_total` from prometheus.

### Sampled WARN logging

When a deployment accumulates more than 100 scope denials within a 60-second window, the host emits a single structured WARN log:

```
WARN scope denial rate threshold crossed for deployment; check scopes configuration
  deployment="/api/my-guest"  category=Kv  denials_per_min=143
```

Subsequent denials in the same window increment counters silently. The threshold resets every 60 seconds.

## Alert recommendations

```promql
# Deployments still running with allow-all scopes
increase(faas_scopes_allow_all_total[5m]) > 0

# Spike in scope denials for any single deployment
increase(faas_scope_denials_total[5m]) > 50

# Linker cache miss rate above 10% (many distinct scope shapes)
rate(faas_linker_cache_miss_total[5m])
  / (rate(faas_linker_cache_hit_total[5m]) + rate(faas_linker_cache_miss_total[5m]))
  > 0.1
```
