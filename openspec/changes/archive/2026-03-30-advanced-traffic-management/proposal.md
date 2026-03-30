# Proposal: Change 024 - Advanced Traffic Management

## Context
To achieve zero-downtime deployments and safe rollouts, Tachyon Mesh must support advanced traffic routing strategies (Blue/Green, Canary, A/B Testing). Currently, the `integrity.lock` maps a route to a single WASM module. We need to evolve this 1-to-1 mapping into a 1-to-Many mapping where the Axum router evaluates weights and HTTP headers to dynamically select the correct WASM guest version at runtime.

## Objective
Update the `RouteConfig` schema to support an array of `Targets`. Implement a traffic-splitting algorithm in the `core-host` Axum handler. Support both probabilistic routing (weights, for Canary/Blue-Green) and deterministic routing (header matching, for A/B Testing and sticky dependency trees).

## Scope
- Update `tachyon-cli` to generate targets with `weight` (0-100) or `match_header` conditions.
- Update `core-host` routing logic:
  - First, evaluate header matches (e.g., if `X-Cohort == beta`, route to V2).
  - Second, if no headers match, use a random number generator to route based on weights.
- Ensure the Outbound IPC / WIT capability (from Change 010) automatically forwards the `X-Tachyon-Cohort` header to maintain context across FaaS dependency chains.

## Success Metrics
- A route configured with a 50/50 weight split effectively distributes 100 requests roughly equally between two different WASM modules.
- A route configured with a header match correctly routes requests containing `X-Cohort: beta` to the beta WASM module, while routing all others to the stable module.
- FaaS A calling FaaS B successfully propagates the cohort header, ensuring the user stays in the same A/B test group throughout the entire microservice transaction.