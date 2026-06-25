## Why

Tachyon detects and validates ModelOpt/NVFP4 checkpoints but still has no
complete architecture runtime for them. The installed
`nvidia--Qwen3.6-35B-A3B-NVFP4` checkpoint is therefore listed by the OpenAI API
but rejected because its actual architecture is `qwen3_5_moe`, not Llama.

This is a concrete specialization of GitHub issue
[#228](https://github.com/astorise/Tachyon-Mesh/issues/228), which tracks broad
Qwen/Gemma/Phi/DeepSeek architecture support but does not specify the hybrid
Qwen 3.5 MoE and mixed FP8/NVFP4 execution contract required here.

## What Changes

- Add a text-generation runtime for the Hugging Face
  `Qwen3_5MoeForConditionalGeneration` /
  `qwen3_5_moe_text` architecture.
- Implement its hybrid sequence of linear-attention and full-attention layers,
  rotary-position behavior, gated projections, RMS normalization, vocabulary
  projection, and autoregressive KV/recurrent state.
- Implement sparse MoE routing for configurable expert counts, top-k experts
  per token, shared experts, and expert aggregation.
- Execute ModelOpt mixed-precision checkpoints where attention projections are
  FP8 and MoE/shared-expert/LM-head weights are W4A16 NVFP4.
- Reuse the existing typed safetensors index, NVFP4 dequantization, native
  CUDA/CUTLASS capability gates, sampling, chat-template, stop, streaming, and
  OpenAI response paths.
- Introduce an architecture descriptor/registry so additional checkpoints can
  use this runtime when their declared architecture, layer semantics,
  quantization metadata, and tensor-name contract are compatible.
- Reject merely “NVFP4” models whose architecture or tensor contract is not
  implemented, with an actionable compatibility error.
- Add deterministic small fixtures and an opt-in probe for the installed
  checkpoint; CI SHALL NOT download large external model artifacts.
- Keep vision inputs and the Qwen vision tower out of scope; those remain
  covered by GitHub issue
  [#238](https://github.com/astorise/Tachyon-Mesh/issues/238).

## Capabilities

### New Capabilities

- `qwen35-moe-runtime`: Hybrid-attention sparse-MoE text generation for
  Qwen 3.5-compatible checkpoints.

### Modified Capabilities

- `ai-inference`: ModelOpt/NVFP4 aliases with a supported architecture execute
  real buffered and streaming generation rather than the unsupported boundary.
- `modelopt-nvfp4-kernels`: Mixed FP8/NVFP4 architecture execution composes the
  existing typed quantized components and capability-gated kernels.
- `ai-layer-wise-inference`: Large compatible MoE checkpoints preserve
  memory-profile semantics without loading every expert and layer onto the
  accelerator simultaneously.

## Impact

- `core-host/src/ai_inference` architecture loading, decode state, batching,
  quantized linear operators, and CUDA integration.
- Potential additions to the `astorise/candle` fork if Candle 0.10 lacks the
  required Qwen 3.5 hybrid-attention or sparse-MoE primitives.
- Model broker compatibility metadata and runtime diagnostics.
- OpenAI `/v1/chat/completions` becomes usable for the installed checkpoint once
  the route binding change is also deployed.
- Significant GPU-memory, host-memory, correctness, and performance testing.
