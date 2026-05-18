# Implementation Tasks

- [x] **Task 1: Reset False-Positive Spec Audits**
  - Locate `openspec/changes/archive/2026-05-17-dynamic-geo-pinning/tasks.md` and replace `[x]` with `[ ]`.
  - Locate `openspec/changes/archive/2026-05-17-cqrs-materialized-views/tasks.md` and replace `[x]` with `[ ]`.
  - Locate `openspec/changes/archive/2026-05-17-baas-advanced-capabilities/tasks.md` and replace `[x]` with `[ ]`.

- [x] **Task 2: Wire Wasmtime Linker Telemetry Import**
  - Edit Wasmtime runtime linkage configurations in `core-host`.
  - Expose the host function `push_custom_metric` to guest execution context matching the WIT signature defined in `wit/telemetry/custom-metrics.wit`.

- [x] **Task 3: Implement GitOps Canary Metrics Loop**
  - Open `systems/system-faas-gitops-broker/src/lib.rs`.
  - Replace static mock responses with an execution loop that parses or validates canary thresholds against live data metrics retrieved from the host registry.

- [ ] **Task 4: Implement Host-Guest Integration Test Suite**
  - Create the test manifest file `core-host/tests/host_guest_integration_test.rs`.
  - Write an integration test executing a guest component, invoking the custom metrics host boundary, and asserting metric state storage persistence on the host side.
