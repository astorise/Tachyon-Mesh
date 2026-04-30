# Implementation Tasks

## Phase 1: Patching the Cache Key
- [x] Locate the cache key generation logic in `core-host/src/main.rs`.
- [x] Retrieve the engine instance and call `.precompile_compatibility_hash()`.
- [x] Append the resulting string to the formatting macro of the `cache_key`.

## Phase 2: Implementing the Startup Purge
- [x] Create a `secure_cache_bootstrap` initialization function.
- [x] Hook this function into the early boot sequence of `core-host` (before any Wasm module is loaded).
- [x] Ensure the `redb` transaction safely drops and clears the `cwasm_cache` bucket if a mismatch is detected.

## Phase 3: Validation & Testing
- [x] **Test Normal Boot:** Start the host twice. Verify the cache is hit on the second boot and no purge occurs.
- [x] **Test Config Change:** Modify the `wasmtime::Config` in the code (e.g., toggle `consume_fuel(true)`). Recompile and run the host. Verify the startup sequence logs the "Purging stale Cwasm cache" warning and forces a safe recompilation of the modules.
- [x] **Safety Audit:** Run `cargo clippy` to ensure no unsafe deserialization occurs without this exact key matching strategy.
