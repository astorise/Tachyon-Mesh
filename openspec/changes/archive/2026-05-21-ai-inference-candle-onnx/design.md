## Context

The previous `ai-inference` implementation used ORT (Microsoft ONNX Runtime) via FFI. ORT links against a native shared library that is not musl-compatible, preventing `core-host --features ai-inference` from building on Alpine. With `Dockerfile.fips` requiring musl for the FIPS variant, and the overall goal of reducing FFI surface area, replacing ORT with a pure-Rust alternative became the right call.

`candle-onnx` (part of the Hugging Face `candle` project) loads and evaluates ONNX `ModelProto` graphs in pure Rust. The `wasmtime-wasi-nn` crate provides traits (`BackendInner`, `BackendGraph`, `BackendExecutionContext`) for plugging in custom backends. A thin implementation layer bridges the two.

`prost` is used to decode raw ONNX bytes into `ModelProto` (ONNX's proto3 schema), matching what `candle-onnx` expects for model loading.

## Goals / Non-Goals

**Goals:**
- Replace ORT with `candle-onnx` as the ONNX inference engine behind WASI-NN.
- Preserve the guest-visible WASI-NN API: `graph_load(bytes, onnx, cpu) → init_execution_context → set_input → compute → get_output`.
- Make `--features ai-inference` musl-compatible so FIPS+AI builds are possible.
- Keep `ai-inference` optional (not in `--all-features` unless explicit).

**Non-Goals:**
- GPU inference support (candle #3491 blocks `simple_eval` on CUDA/Metal — deferred).
- Batching scheduler changes (existing scheduler wiring unchanged).
- LoRA adapter support (pre-existing spec; unchanged by this substitution).

## Decisions

### D1: candle-onnx over ort

`candle-onnx` is pure Rust, musl-compatible, and avoids native library linking. `ort` provides a richer operator set and GPU support but requires a native ONNX Runtime shared library (incompatible with musl and FROM-scratch images). Given current CPU-only usage, the operator coverage of `candle-onnx` is sufficient.

Alternative considered: `tract-onnx` (pure Rust, good operator coverage). Rejected because `candle-onnx` integrates naturally with the existing candle ecosystem already present for LoRA work.

### D2: Implement wasmtime-wasi-nn traits directly

Rather than wrapping candle-onnx in a generic inference abstraction, `CandleOnnxBackend` implements `BackendInner`, `CandleOnnxGraph` implements `BackendGraph`, and `CandleOnnxContext` implements `BackendExecutionContext` directly. This avoids an extra indirection layer and keeps the code minimal (~160 lines).

### D3: EmptyGraphRegistry for byte-based model loading

WASI-NN preview1 guests load models by passing raw bytes via `graph_load`, not by registry name lookup. `EmptyGraphRegistry` (a no-op `GraphRegistry` impl) is passed to `WasiNnCtx::new()` so that the host doesn't need a preloaded model catalog. Models arrive as raw bytes at graph load time and are decoded via `prost::Message::decode::<ModelProto>()`.

### D4: CPU-only until candle #3491 is fixed

`candle_onnx::simple_eval` is hardcoded to use the CPU device. GPU inference would require the upstream fix in candle. This is acceptable for the initial candle-onnx integration; GPU path can be enabled without API changes once the fix lands.

### D5: prost in [dependencies] not [dev-dependencies]

`ModelProto::decode` is called at runtime (not just in tests), so `prost` must be a runtime dependency. It is gated behind `#[cfg(feature = "ai-inference")]` to avoid bloating non-AI builds.

## Risks / Trade-offs

- **Operator coverage gap** → `candle-onnx` supports core ONNX ops but not the full operator set. Complex models may fail at runtime with an "unsupported op" error. Mitigation: document which ONNX op subsets are supported; test against the target models.
- **GPU inference blocked** → CPU-only until candle #3491. Mitigation: track the upstream issue; the fix is expected to be a one-line device parameter change.
- **prost version drift** → `prost` version must stay aligned with candle-onnx's internal proto definitions. Mitigation: pin `prost = "0.14"` and update alongside candle-onnx upgrades.
- **candle-onnx is pre-1.0** → API may change across minor releases. Mitigation: pin minor version; review release notes on upgrade.

## Migration Plan

1. Delete `ort` from `core-host/Cargo.toml` and all associated FFI code.
2. Add `candle-core`, `candle-nn`, `candle-onnx`, `prost` as optional `ai-inference` deps.
3. Implement `candle_onnx_backend.rs` with the three wasmtime-wasi-nn trait impls.
4. Restore `WasiNnCtx` wiring in `runtime_types.rs`, `component_hosts.rs`, `guest_runtime.rs`.
5. Add `protobuf-compiler` to CI system deps.
6. Verify `cargo check -p core-host --features ai-inference` passes.

Guests using `wasi-nn` witx API require no changes — the host-side substitution is transparent.

## Open Questions

- GPU path: monitor candle #3491 (issue: `simple_eval` ignores the `device` parameter). When fixed, thread the `ExecutionTarget` from the guest through to the candle eval call.
- Operator coverage: which ONNX ops does the `guest-ai` model require? Should be audited against candle-onnx's `src/ops/` coverage table.
