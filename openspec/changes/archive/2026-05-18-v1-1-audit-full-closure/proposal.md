# Proposal: v1.1.x Audit Full Closure & Feature Completion

## Context
This is the final phase of the audit remediation process for the `v1.1.x` branch. While security flaws (P0) and hygiene/SemVer alignment have been covered, this final pass achieves complete audit compliance by:
1. **Correcting the Remaining Ghost Specifications:** Reverting the false-positive complete status (`[x]`) on the 3 remaining specifications (`dynamic-geo-pinning`, `cqrs-materialized-views`, and `baas-advanced-capabilities`).
2. **Finishing a High-Value Feature Set:** As recommended by the audit, completing the end-to-end integration of the Custom Metrics + Canary orchestration feature. This requires adding a Wasmtime host import link for `push_custom_metric` so guests can invoke it, and completing the evaluation stub in `gitops-broker`.
3. **Establishing Integration Testing:** Introducing initial end-to-end host-to-guest integration test boundaries for the newly added components.

## Objective
- Uncheck false-positive execution logs in the 3 remaining stubs.
- Bind `push_custom_metric` from `core-host/src/telemetry/mod.rs` to the Wasmtime Linker structure inside `core-host`.
- Wire the generic PromQL processing task logic in `systems/system-faas-gitops-broker/src/lib.rs`.
- Scaffold an integration test validating `host ↔ guest` telemetry exchange.

## Scope
- `openspec/changes/archive/2026-05-17-dynamic-geo-pinning/tasks.md`
- `openspec/changes/archive/2026-05-17-cqrs-materialized-views/tasks.md`
- `openspec/changes/archive/2026-05-17-baas-advanced-capabilities/tasks.md`
- `core-host/src/host_core/guest_runtime.rs` (or Wasmtime Linker setup code)
- `systems/system-faas-gitops-broker/src/lib.rs`
- `core-host/tests/host_guest_integration_test.rs`