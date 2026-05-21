## Why

ORT (ONNX Runtime) was removed because its musl-incompatibility blocked FIPS+musl builds, its FFI surface added complexity, and the ROI did not justify maintaining a non-pure-Rust dependency. `candle-onnx` (Hugging Face, pure Rust) provides equivalent ONNX model loading and CPU inference while remaining musl-compatible and eliminating the native library FFI overhead. WASI-NN integration is preserved by implementing `wasmtime-wasi-nn` traits (`BackendInner`, `BackendGraph`, `BackendExecutionContext`) on top of `candle-onnx`, giving guests the same `graph_load → init_execution_context → set_input → compute → get_output` API.

## What Changes

- **ORT removed**: `ort` crate dependency and all related FFI code deleted from `core-host`.
- **candle-onnx added**: `candle-core`, `candle-nn`, `candle-onnx`, `prost` added to `ai-inference` feature deps in `core-host/Cargo.toml`.
- **CandleOnnxBackend**: new `core-host/src/ai_inference/candle_onnx_backend.rs` implementing `BackendInner + BackendGraph + BackendExecutionContext` for WASI-NN.
- **`ai-inference` now musl-compatible**: pure Rust implementation eliminates the prior musl blocker.
- **GPU inference deferred**: candle-onnx issue #3491 (`simple_eval` hardcoded to CPU) means GPU path waits for upstream fix; CPU-only for now.
- **CI `--all-features` now requires `protobuf-compiler`**: `prost` (used for `ModelProto::decode`) pulls in protobuf support.

## Capabilities

### New Capabilities

*(none — this replaces an existing backend without adding new guest-visible capability)*

### Modified Capabilities

- `ai-inference`: ORT backend replaced by candle-onnx; WASI-NN guest API preserved; musl compatibility gained; GPU inference deferred pending upstream candle fix.

## Impact

- **`core-host/src/ai_inference/candle_onnx_backend.rs`** (new ~160 lines): `CandleOnnxBackend`, `CandleOnnxGraph`, `CandleOnnxContext` structs.
- **`core-host/src/ai_inference.rs`**: `build_wasi_nn_ctx()` now returns `CandleOnnxBackend`; `EmptyGraphRegistry` added for byte-based model loading.
- **`core-host/Cargo.toml`**: `ort` removed; `candle-core`, `candle-nn`, `candle-onnx`, `prost` added as optional `ai-inference` deps.
- **`core-host/src/host_core/`**: `runtime_types.rs`, `component_hosts.rs`, `guest_runtime.rs` restored `WasiNnCtx` field and linker registration under `cfg(feature = "ai-inference")`.
- **`.github/workflows/ci.yml`**: `protobuf-compiler` added to system deps (needed for `prost`).
- No WIT interface changes — guests use the same `wasi-nn` witx API.
