# Proposal: Change 023 - AI Inference & Hardware Acceleration (WASI-NN)

## Context
Running AI/ML inference (like LLMs or image classification) in traditional serverless environments is painfully slow due to container cold starts (loading gigabytes of CUDA drivers and Python environments). WebAssembly solves the cold start, but its strict sandbox prevents direct access to GPU hardware. The Bytecode Alliance created the `WASI-NN` standard to bridge this gap, allowing a lightweight WASM guest to delegate tensor computations to the Host's optimized Machine Learning backends (ONNX, OpenVINO, CoreML).

## Objective
Integrate the `wasmtime-wasi-nn` crate into the `core-host`. We will implement it behind a Cargo feature flag (`ai-inference`) because it requires native ML C++ libraries on the host system. We will create a `guest-ai` FaaS that uses the WASI-NN guest bindings to perform a simple inference (e.g., image classification or text embeddings) using a model loaded by the host.

## Scope
- Add `wasmtime-wasi-nn` to `core-host` as an optional dependency.
- Configure the Wasmtime linker to expose the WASI-NN preview1 imports to legacy guests when `ai-inference` is enabled.
- Set up an ONNX backend in the host without changing the default build.
- Create a `guest-ai` module in Rust that imports `wasi-nn`, loads an ONNX model from a sealed read-only `/models` directory, sets an input tensor, calls `compute()`, and reads the output tensor.
- Use sealed route volume mounts in `integrity.lock` to define which model directory is available to the AI FaaS.

## Success Metrics
- The `core-host` compiles successfully with the `ai-inference` feature.
- A `curl` request containing tensor JSON is sent to `/api/guest-ai`.
- `guest-ai` successfully passes the tensor to the host's WASI-NN implementation.
- The host executes the inference on its native ONNX backend and returns the output tensor to the guest, which returns it as JSON.
