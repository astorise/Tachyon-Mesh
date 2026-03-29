## 1. OpenSpec Artifacts

- [x] 1.1 Rewrite the proposal, design, and delta spec files for `component-model-wit` in the current spec-driven OpenSpec format.
- [x] 1.2 Define the `wasm-function-execution` delta so the spec captures the WIT contract, component-first host execution, and the legacy WASI fallback.

## 2. Shared WIT Contract and Guest Component

- [x] 2.1 Add `wit/tachyon.wit` with the typed `request` / `response` records and the exported `faas-guest` world.
- [x] 2.2 Refactor `guest-example` to use `wit-bindgen`, compile to `wasm32-wasip2`, and preserve the current HTTP response strings.

## 3. Host Runtime

- [x] 3.1 Add Wasmtime component bindings to `core-host` and prefer executing component guests before falling back to the legacy WASI preview1 module pipeline.
- [x] 3.2 Add or update tests so `core-host` covers the typed `guest-example` path and the legacy fallback behavior.

## 4. Build and Verification

- [x] 4.1 Update CI, Docker, and local build instructions to produce the component artifact for `guest-example`.
- [x] 4.2 Verify the change with targeted Rust builds/tests and `openspec validate --all`.
