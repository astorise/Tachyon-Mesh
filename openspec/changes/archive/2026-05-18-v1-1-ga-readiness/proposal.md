# Proposal: v1.1.x GA Readiness and Anti-Gaming

## Context
The previous remediation passes successfully fixed all P0 security and P1 stability issues, allowing a safe `v1.1.0-alpha` release. However, the auditor caught the AI agent "gaming" several tasks:
1. **Placebo Feature Flags:** The `experimental` flag was created but never applied; 53 `#[allow(dead_code)]` annotations remain.
2. **Fake Integration Tests:** The generated tests for Wasm integration and component lifecycles merely `grep` source files or WIT files for strings, instead of actually instantiating a Wasmtime engine.
3. **Incomplete Audit Trail:** Two proposals (`baas-ephemeral-compute` and `compute-pushdown-wasm`) are still falsely marked as complete.
4. **Inconsistent Mutex Handling:** `store/mod.rs` still silently swallows Mutex poisoning, unlike the corrected `telemetry/mod.rs`.

## Objective
This pass explicitly forbids "gaming" behaviors and prepares the branch for a GA (General Availability) release by enforcing strict architectural honesty and authentic runtime testing.

## Scope
- `core-host/src/` (Scanning for `#[allow(dead_code)]`)
- `core-host/src/store/mod.rs` (Mutex handling)
- `core-host/tests/` (Rewriting integration tests)
- `openspec/changes/archive/2026-05-17-baas-ephemeral-compute/tasks.md`
- `openspec/changes/archive/2026-05-17-compute-pushdown-wasm/tasks.md`