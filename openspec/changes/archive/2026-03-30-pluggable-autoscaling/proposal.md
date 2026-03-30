# Proposal: Change 021 - Pluggable Legacy Autoscaling (System FaaS)

## Context
Legacy containers (Java/COBOL) behind our sidecars require horizontal autoscaling when the Tachyon asynchronous queue (Semaphore) fills up. Instead of forcing a single scaling strategy (e.g., relying entirely on external tools like KEDA, or hardcoding K8s API calls in the Rust host), we will leverage our "System FaaS" architecture (from Change 015) to make autoscaling entirely pluggable by the end-user via the `integrity.lock` configuration.

## Objective
Provide two distinct System FaaS modules for legacy autoscaling, allowing the user to choose their preferred strategy (or none at all).
1. `system-faas-keda`: Exposes queue metrics via an HTTP endpoint for KEDA to scrape.
2. `system-faas-k8s-scaler`: A fully autonomous background task that reads the queue metrics and pushes scaling commands directly to the Kubernetes API Server.

## Scope
- Expand the `tachyon.wit` interface to allow System FaaS to read the exact state of the routing semaphores (pending queue size).
- Add a background "tick" mechanism in the `core-host` that periodically invokes a specific exported function (`export on-tick: func();`) for any System FaaS that requires background execution.
- Implement the two WASM modules.
- Update the documentation to explain how users can select the module in their GitOps workflow.

## Success Metrics
- If no scaling FaaS is configured, the Host incurs zero scaling overhead.
- If `system-faas-keda` is configured, a `GET /metrics/scaling` returns the queue size in Prometheus format.
- If `system-faas-k8s-scaler` is configured, the Host triggers it every 5 seconds, and the WASM guest successfully makes an outbound HTTP request (via the Host outbound capability) to a mock K8s API.