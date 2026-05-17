# Proposal: Business & Cognitive Canary Orchestration

## Why
Tachyon's Canary release strategies rely on standard L4/L7 health signals (HTTP 5xx, latency). However, modern deployments—especially AI workloads and e-commerce frontends—require rollbacks based on qualitative signals. An LLM returning a fast hallucination (HTTP 200) or a UI update causing a 30% drop in conversions cannot be detected by standard infrastructure metrics.

1. **Blind Rollouts:** The Canary engine cannot evaluate the actual business or cognitive impact of a deployment, leading to potentially catastrophic product regressions being marked as "Healthy."
2. **Integration Nightmare:** Hardcoding support for external analytics platforms (Google Analytics, Piano) or AI evaluation suites (Nebula) into the core-host would violate Tachyon's lightweight architectural principles.

## What Changes
Introduce a new WIT contract allowing isolated Wasm components (acting as bridges) to push custom, domain-specific metrics into the core-host's native Prometheus registry.
1. **The WIT Interface:** Implement `tachyon:telemetry/custom-metrics` exposing `push(metric)`.
2. **Prometheus Translation:** The core-host dynamically translates these calls into Prometheus `Counter`, `Gauge`, or `Histogram` instances and exposes them on the standard metrics port (`:9090`).
3. **Canary Evaluation:** Update the `system-faas-gitops-broker` (Canary engine) to parse custom metric thresholds from the deployment manifest (e.g., `piano.checkout_conversion > 12%`) and evaluate them against the local Prometheus registry before advancing rollout steps.

## Impact
- **AI-Safe Releases:** Enables "Cognitive Rollbacks," halting model deployments immediately if hallucination rates spike.
- **Revenue Protection:** Enables business-driven CD, rolling back frontend code if critical conversion flows drop.
