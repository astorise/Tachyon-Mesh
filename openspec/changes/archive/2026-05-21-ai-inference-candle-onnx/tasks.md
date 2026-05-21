## 1. ORT Removal

- [x] 1.1 Remove `ort` from `core-host/Cargo.toml` dependencies
- [x] 1.2 Delete all ORT-related FFI code from `core-host/src/ai_inference.rs`
- [x] 1.3 Remove `WasiNnBackend` abstraction and `WasiTensor` types that wrapped ORT

## 2. candle-onnx Backend

- [x] 2.1 Add `candle-core`, `candle-nn`, `candle-onnx`, `prost` as optional `ai-inference` deps in `core-host/Cargo.toml`
- [x] 2.2 Create `core-host/src/ai_inference/candle_onnx_backend.rs`
- [x] 2.3 Implement `CandleOnnxBackend` with `BackendInner` trait (encoding=Onnx, load via prost ModelProto::decode)
- [x] 2.4 Implement `CandleOnnxGraph` with `BackendGraph` trait (init_execution_context)
- [x] 2.5 Implement `CandleOnnxContext` with `BackendExecutionContext` trait (set_input, compute via simple_eval, get_output)
- [x] 2.6 Add `wasi_to_candle()` and `candle_to_wasi()` tensor conversion helpers

## 3. WASI-NN Host Wiring Restoration

- [x] 3.1 Add `EmptyGraphRegistry` (no-op `GraphRegistry` impl) to support byte-based model loading
- [x] 3.2 Restore `build_wasi_nn_ctx()` in `core-host/src/ai_inference.rs` returning `CandleOnnxBackend`
- [x] 3.3 Restore `wasi_nn: WasiNnCtx` field in `LegacyHostState` (runtime_types.rs) under `cfg(ai-inference)`
- [x] 3.4 Restore `LegacyHostState::new` `ai_runtime` parameter in `component_hosts.rs`
- [x] 3.5 Restore `wasmtime_wasi_nn::witx::add_to_linker` call in `guest_runtime.rs` under `cfg(ai-inference)`

## 4. Build + CI

- [x] 4.1 Verify `cargo check -p core-host --features ai-inference` passes
- [x] 4.2 Verify `cargo clippy -p core-host --features ai-inference -- -D warnings` passes
- [x] 4.3 Move `prost` from `[dev-dependencies]` to `[dependencies]` (runtime decode)
- [x] 4.4 Confirm `protobuf-compiler` is in CI system deps (added in fips-musl-alpine change)
- [x] 4.5 Verify `cargo check -p core-host --features ai-inference` step in ci.yml passes in CI

## 5. Documentation

- [x] 5.1 Update `openspec/specs/ai-inference/spec.md` to reflect candle-onnx backend
- [x] 5.2 Note GPU inference deferral (candle #3491) in design docs
