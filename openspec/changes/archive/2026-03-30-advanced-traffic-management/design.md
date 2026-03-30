# Design: Advanced Traffic Management

## Overview
This change extends sealed routes from a single implicit guest module to an
ordered list of explicit traffic targets. The host keeps route-level
concurrency, telemetry, and integrity validation unchanged, but chooses a guest
artifact per request by evaluating deterministic header matches before falling
back to weighted rollout selection.

## Route Model
Each sealed route continues to own the public HTTP path, role, scaling, secret
grants, and volumes. A new optional `targets` array can be attached to the
route:

- If `targets` is empty, the existing path-derived guest resolution remains the
  default behavior for backward compatibility.
- If `targets` contains entries, each entry declares a `module`, an optional
  `match_header`, and a `weight`.
- Header-matched targets are evaluated in declaration order.
- If no target matches by header, the host randomly selects from targets with
  `weight > 0`.
- If every explicit target is header-only and none match, the host falls back
  to the legacy path-derived module so existing route layouts remain valid.

## CLI Shape
The CLI keeps `--route` and `--system-route` as the source of sealed paths, then
adds repeated `--route-target` overrides to attach module choices to an existing
route.

Syntax:

`/path=module[,weight=80][,header=X-Cohort=beta]`

Examples:

- `/api/checkout=checkout-v1,weight=90`
- `/api/checkout=checkout-v2,weight=10`
- `/api/checkout=checkout-beta,header=X-Cohort=beta`

This keeps advanced routing additive and backward compatible with existing
manifest-generation flows.

## Host Request Flow
For each inbound request, the host:

1. Normalizes the route path and resolves the sealed route.
2. Selects the target module using ordered header matching, then weighted
   rollout.
3. Executes the selected module while preserving the existing route-scoped
   concurrency limiter.
4. Propagates cohort headers to outbound mesh fetch hops so downstream routes
   can keep the caller in the same experiment bucket.

## Cohort Propagation
The host treats `X-Cohort` and `X-Tachyon-Cohort` as the request-scoped rollout
context. When either header is present inbound, outbound host-managed requests
forward `X-Tachyon-Cohort`, and they also preserve `X-Cohort` for compatibility
with routes that still match on the shorter name.

## Validation Strategy
Coverage focuses on:

- CLI normalization for explicit route targets.
- Header-first routing decisions.
- Weighted routing decisions.
- Cohort propagation on host-managed mesh fetch requests.
- Backward compatibility for older manifests that do not declare `targets`.
