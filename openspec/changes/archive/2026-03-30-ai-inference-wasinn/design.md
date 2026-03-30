# Design: Change 023 - AI Inference via WASI-NN

## Summary
This implementation adds AI inference support without disturbing the default host build. The host
keeps its existing preview1 legacy execution path and only links `wasi_ephemeral_nn` when built
with `--features ai-inference`. The new `guest-ai` module stays in the legacy guest pipeline so it
can use the existing `wasi-nn` Rust bindings crate directly.

## Host strategy
- Keep `core-host` default features unchanged.
- Add `wasmtime-wasi-nn` as an optional dependency using the ONNX backend.
- Extend `LegacyHostState` with a `WasiNnCtx` only when `ai-inference` is enabled.
- Register the preview1 `wasi_ephemeral_nn` imports in the legacy linker only for that feature.
- Return an explicit runtime error if `guest-ai` is invoked while the feature is disabled, instead
  of surfacing an opaque import-resolution failure.

## Guest strategy
- Implement `guest-ai` as a preview1 guest compiled for `wasm32-wasip1`.
- Accept a JSON payload with `model`, `shape`, `values`, and `output_len`.
- Read an ONNX file from the sealed `/models` directory and call `wasi-nn` using
  `GraphBuilder::build_from_bytes`.
- Return a JSON payload containing the model name, output tensor values, and raw output byte count.

## Model availability
Model access is defined by the sealed route volume, not by an ambient host directory. Operators seal
the route with a read-only mount such as:

```bash
cargo run -p tachyon-cli -- generate --route /api/guest-ai --volume /api/guest-ai=/absolute/path/to/models:/models:ro --memory 64
```

This keeps model exposure explicit in `integrity.lock` and reuses the existing integrity manifest
shape instead of inventing a second model-registry schema for this iteration.

## CI strategy
- Build `guest-ai` as part of the normal artifact matrix.
- Keep the default `core-host` release build unchanged.
- Add an explicit `cargo check -p core-host --features ai-inference` step to validate the optional
  compilation path on GitHub Actions.
