## Why

The archived Magnetar cutover claimed a completed full cutover while Tachyon still used a local fail-closed facade and retained Candle-oriented specs and GPU CI gates. Magnetar now exposes real production Qwen loading, caller-supplied generation requests, streaming events, and device-resident CUDA multi-token decode, so Tachyon can replace the facade with the public Magnetar ingestion/runtime/provider APIs.

## What Changes

- Replace Tachyon's local Magnetar facade with the real Magnetar production Qwen loading path pinned to Magnetar commit `b235783abb2b0c92843febfa3ff29f30745e1934`.
- Use `ProductionModelSource::authorized_local_bundle(ModelArtifactSource::Tachyon(..), root)` and `HuggingFaceIngestor` for Tachyon-staged Qwen bundles instead of parsing `config.json`, tokenizer, or Safetensors payloads in Tachyon.
- Execute production Qwen through Magnetar's real tokenizer, `ModelTrustStore`, `ModelInstance`, Qwen Component, prepared execution plans, and CPU/CUDA Providers.
- Remove permanent Tachyon-owned fake Magnetar constructs such as local `PreparedKernelId`, `TensorId`, hardcoded `CUDA_0`, hardcoded VRAM/dtype capability advertisements, `TACHYON_MAGNETAR_CUDA` as hardware truth, and pseudo `MagnetarArena` residency.
- Enforce fail-closed placement: an explicitly requested CUDA route must not silently fall back to Reference CPU. CUDA multi-token generation is allowed only through Magnetar's real `CudaProvider` and device-resident decode path.
- Make dynamic lazy loading target-aware so requested CPU/GPU placement must match the actual loaded Magnetar binding.
- Remove historical local inference compatibility from the active WIT/spec surface: LoRA adapter injection, per-call layer-wise memory profiles, handle-based layer execution, and local multi-device topology validation.
- Keep the model broker format-neutral: it verifies and installs artifacts, writes provenance, publishes Magnetar-owned upload events, and does not detect GGUF/Safetensors or prewarm tenant LoRA adapters.
- Replace Candle-oriented CUDA CI assertions with Magnetar production-loading CPU coverage and real GPU CUDA prefill coverage that cannot pass with zero selected tests.
- Correct the prior archived cutover record by adding an explicit follow-up change rather than pretending the facade-based cutover was complete.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `ai-inference`: Local Qwen inference changes from a Candle/facade boundary to the real Magnetar production ingestion, trust, tokenizer, ModelInstance, Provider, and generation path.
- `github-actions`: GPU quality gates must run Magnetar CUDA production-loading checks and must fail when no GPU-critical tests are selected or no required hardware assertions actually ran.
- `hardware-capabilities`: Mesh routing must consume real Magnetar Provider/Device metadata instead of Tachyon-fabricated Magnetar capability advertisements.

## Impact

- Affected code: `core-host/Cargo.toml`, `core-host/src/ai_inference.rs`, `core-host/src/ai_inference/magnetar_runtime.rs`, `systems/system-faas-model-broker`, AI inference tests, WIT contracts, and CI workflow commands.
- Affected dependencies: add Magnetar crates from the pinned Magnetar revision: `magnetar-runtime` with `wasmtime-component-engine`, `magnetar-loader-huggingface`, `magnetar-provider-cpu`, and conditionally `magnetar-provider-cuda`.
- Affected behavior: `magnetar:` Qwen bindings load through Magnetar production ingestion; non-Qwen, untrusted, unavailable CUDA, and unsupported request fields fail explicitly; Reference CPU may be used only when placement policy allows CPU.
- Affected documentation/specs: canonical AI inference and GPU CI requirements must stop describing Candle as the active local inference path.
