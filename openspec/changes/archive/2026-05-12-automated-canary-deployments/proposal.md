# Proposal: Automated Canary Deployments for WASM Components

## Problems

Currently, when a new WASM component or AI model version is applied via the `Seal & Apply` pipeline, traffic shifts immediately (or requires tedious manual routing adjustments). If a new version contains a logic bug or causes a regression in inference accuracy, 100% of the active traffic is affected until a manual rollback is triggered. 

## What Changes

We will introduce a native **Canary Deployment Strategy**. This enables gradual, percentage-based traffic shifting to the new WASM component version, fully automated by a background evaluator.

1. **Manifest Expansion:** Introduce a `strategy` block within the component deployment configuration (in WIT) to define `step-weight`, `interval`, and `error-threshold`.
2. **Fractional Traffic Shifting:** Enhance the core host's traffic router to support fractional probability weighting between `v-current` and `v-next`.
3. **Telemetry-Driven Evaluation:** Create a background worker that polls `telemetry::TelemetrySnapshot` during a canary rollout. If the error rate (HTTP 5xx or execution panics) for `v-next` exceeds the `error-threshold`, an automatic rollback to `v-current` is instantly triggered.
4. **UI Observability:** Update `TachyonWorkloadsPanel` to display a live progress bar representing the canary rollout status and traffic split.