# Tasks: Change 023 Implementation

- [x] 1.1 Add the `ai-inference` feature and optional `wasmtime-wasi-nn` dependency to `core-host`,
  wiring the preview1 linker only when the feature is enabled.
- [x] 1.2 Add a new legacy `guest-ai` crate that parses JSON tensor requests, loads an ONNX model
  from a sealed `/models` directory, and returns the output tensor as JSON.
- [x] 1.3 Keep the default host build unchanged while returning a clear runtime error when
  `/api/guest-ai` is invoked without `--features ai-inference`.
- [x] 1.4 Update the README, Docker build, and GitHub Actions workflow to build `guest-ai` and
  validate `cargo check -p core-host --features ai-inference`.
- [x] 1.5 Replace the invalid change artifacts with valid OpenSpec deltas and document the sealed
  model-volume approach used for AI inference.
