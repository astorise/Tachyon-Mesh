# Specification: GA Readiness

## 1. Truthful Audit Trail
The following proposals must have their implementation tasks reset from `[x]` to `[ ]` as they lack active wiring:
- `baas-ephemeral-compute`
- `compute-pushdown-wasm`

## 2. Enforcing Real Feature Flags (No Placebo)
The agent previously ignored the instruction to swap dead code attributes.
- **Action:** Scan `core-host/src` for all occurrences of `#[allow(dead_code)]`. 
- **Action:** Remove `#[allow(dead_code)]` completely.
- **Action:** Replace it with `#[cfg(feature = "experimental")]`.

## 3. Real Wasmtime Execution Test (Anti-Grep Rule)
The current integration tests reading `.rs` or `.wit` files are fake.
- **Action:** Delete the grep-based fake tests (`cdc_broadcaster_test.rs`, `media_server_test.rs`, `sql_engine_test.rs`, `vector_search_test.rs`, `view_builder_test.rs`, `olap_engine_test.rs`, and `host_guest_integration_test.rs`).
- **Action:** Create a single, real test file: `core-host/tests/real_wasm_integration_test.rs`.
- **Action:** This test MUST instantiate a Wasmtime `Engine`, `Store`, and `Linker`. To avoid missing `.wasm` artifacts, use an inline WebAssembly Text format (`.wat`) string that imports the telemetry boundary and executes a dummy function. The test must execute the guest and assert the host state changes.

## 4. Consistent Mutex Poisoning
- **Action:** In `core-host/src/store/mod.rs` (around lines 249, 258), replace `unwrap_or_else(|p| p.into_inner())` with a block that logs a `tracing::warn!` before returning the inner data, matching the pattern established in `telemetry/mod.rs`.