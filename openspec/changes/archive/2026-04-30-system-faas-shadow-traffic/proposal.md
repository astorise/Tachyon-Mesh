# Proposal: Opt-in Shadow Traffic Proxy

## Context
Enterprise users need to test new FaaS versions (or new Foundation Models) with real production traffic without impacting the client. Building this into the `core-host` would add unnecessary complexity and latency. Because Tachyon Mesh already possesses a robust asynchronous IPC and Event Bus, Shadow Traffic is simply a routing pattern that can be offloaded entirely to an optional System FaaS.

## Proposed Solution
We will create `system-faas-shadow-proxy` and utilize a "Fire-and-Forget" pattern:
1. **Config:** The route in `integrity.lock` specifies a `shadow_target` alongside the primary target.
2. **Core-Host (Zero Blocking):** The host routes the request to the primary FaaS and streams the response back to the client. *After* the response is sent (or in parallel), the host drops an event onto the async bus: `ShadowTask { request_clone, primary_response_hash, shadow_target }`.
3. **The System FaaS:** `system-faas-shadow-proxy` picks up the event, executes the request against the `shadow_target`, and compares the result with the `primary_response_hash`.
4. **Metrics:** Divergences (mismatched payloads or status codes) are sent to `system-faas-otel`.

## Objectives
- Provide Enterprise-grade Traffic Mirroring with strictly **zero latency impact** on the primary user request.
- Keep the `core-host` clean by treating Shadow Traffic as a standard asynchronous System FaaS workload.