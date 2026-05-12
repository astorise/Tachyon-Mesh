# Tasks

## 1. WIT Contracts & Domain Types
- [x] Update `wit/config-workloads.wit` to add a `deployment-strategy` variant (Rolling, Canary) and a `canary-config` record (`step-weight: u32`, `interval-secs: u64`, `max-error-rate: float32`).
- [x] Regenerate bindings using `wit_bindgen!` and map these to `core-host/src/host_core/domain_types.rs`.

## 2. Core Host (Routing & Evaluation)
- [x] Modify `core-host/src/host_core/app_runtime.rs` (or the specific routing module) to evaluate randomized fractional routing when a canary state is active.
- [x] Implement `canary_evaluator` in `core-host/src/host_core/background_workers.rs`. It must loop at the configured `interval-secs`, increase the traffic fraction by `step-weight`, and check the `TelemetrySnapshot` for the component.
- [x] Implement the automatic rollback logic inside the evaluator if `error_rate > max-error-rate`.

## 3. UI Implementation
- [x] Update `tachyon-ui/src/components/domains/TachyonWorkloadsPanel.ts` to expose the new Canary configuration fields in the deployment forms.
- [x] Add a visual indicator (e.g., a progress bar or split pie chart) in the Workloads overview for instances currently undergoing a Canary rollout, using live data from the metrics endpoint.