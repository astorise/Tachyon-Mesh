## Why

Tachyon's native Candle text-generation runtime accepts Llama checkpoints and a
focused set of MoE paths, but rejects the other architecture families already
available in `candle_transformers`. This leaves the model catalog effectively
Llama-only and blocks the high-demand Qwen and Gemma families tracked by GitHub
issue [#228](https://github.com/astorise/Tachyon-Mesh/issues/228).

## What Changes

- Introduce an explicit architecture registry that normalizes Hugging Face
  `config.json` and GGUF metadata before selecting a model backend.
- Add native safetensors execution for Qwen2/Qwen3 dense and Gemma2/Gemma3
  text-generation checkpoints as the first delivery tranche.
- Add Phi3/Phi4 and DeepSeek V2/V3/R1 integration as follow-up families behind
  the same backend contract when their required Candle primitives are present.
- Extend GGUF architecture recognition only for families with a verified
  quantized loader; recognized-but-unimplemented combinations fail closed.
- Preserve existing sampling, chat-template, stop-sequence, buffered,
  streaming, batching, and sealed-alias behavior across architecture backends.
- Validate each architecture against its required config fields and tensor
  contract, returning actionable typed errors for unsupported variants.
- Define compatibility expectations for single-device execution and explicitly
  gate tensor-, pipeline-, and expert-parallel modes per architecture rather
  than silently routing through Llama-specific engines.
- Keep the archived Qwen 3.5 hybrid-MoE ModelOpt/NVFP4 runtime as a separate
  specialized capability; this change does not replace its quantized backend.

## Capabilities

### New Capabilities

- `candle-architecture-runtime`: Architecture discovery, backend registration,
  family-specific loading, generation, compatibility validation, and phased
  support for Qwen, Gemma, Phi, and DeepSeek checkpoints.

### Modified Capabilities

- `ai-inference`: Native text generation must dispatch supported non-Llama
  checkpoints to architecture-specific Candle backends while preserving the
  existing request, streaming, security, and unsupported-model contracts.

## Impact

- `core-host/src/ai_inference/candle_llm_runtime.rs` model probing, loaded-model
  dispatch, generation state, and error reporting.
- New family-specific adapters under `core-host/src/ai_inference/`, plus tests
  and tiny deterministic checkpoint fixtures.
- `candle-transformers` usage and potentially the `astorise/candle` fork for
  reusable architectures not yet available upstream.
- Model-format documentation and CI coverage for optional `ai-inference` and
  `candle-cuda` builds.
- No API or manifest breaking change; existing Llama, Mixtral, Qwen 3.5 MoE,
  ModelOpt/NVFP4, ONNX, and mock boundaries remain available.
