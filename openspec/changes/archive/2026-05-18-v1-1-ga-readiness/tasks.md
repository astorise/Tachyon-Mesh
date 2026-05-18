# Implementation Tasks

- [x] **Task 1: Exhaustive Audit Trail Reset (Anti-Gaming)**
  - Open `openspec/changes/archive/2026-05-17-baas-ephemeral-compute/tasks.md` and uncheck task 4.
  - Open `openspec/changes/archive/2026-05-17-compute-pushdown-wasm/tasks.md` and uncheck task 4.
  - Scan the recent `audit-remediation` change directories (e.g., `2026-05-18-v1-1-audit-remediation`, `full-closure`, `absolute-polish`) and explicitly change `[x]` to `[ ]` for the tasks related to "Experimental Feature Flagging" (T5), "Integration Test Suite" (T4), and "Lifecycle Tests" (T4/T1) that were faked with grep.

- [x] **Task 2: Hard-Swap `allow(dead_code)` to `cfg(feature = "experimental")`**
  - Search globally in `core-host/src/` for `#[allow(dead_code)]`.
  - Delete `#[allow(dead_code)]` entirely and inject `#[cfg(feature = "experimental")]` in its exact place. Do not leave any `allow(dead_code)` placebo behind.

- [x] **Task 3: Implement Authentic Wasmtime E2E Test**
  - Delete the fake grep-based tests in `core-host/tests/` (e.g., `cdc_broadcaster_test.rs`, `media_server_test.rs`, `host_guest_integration_test.rs`, etc.).
  - Create a single file `core-host/tests/real_wasm_integration_test.rs`.
  - Write a real test using `wasmtime::Module::new(engine, r#" (module (func (export "run"))) "#)` (inline WebAssembly text) to prove the host runtime instantiates correctly without relying on source file grepping.

- [x] **Task 4: Fix Store Mutex Poisoning Consistency**
  - Open `core-host/src/store/mod.rs`.
  - Locate `Mutex::lock().unwrap_or_else(|p| p.into_inner())` (around lines 249, 258).
  - Modify the closure to log `tracing::warn!("Store Mutex poisoned, recovering data");` before returning the inner data, aligning it with the telemetry module's behavior.