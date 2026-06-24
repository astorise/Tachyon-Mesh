## Context

The current runtime has three separate pieces that do not yet form an
end-to-end NVFP4 text-generation engine:

- `candle_llm_runtime.rs` implements buffered and streaming generation, chat
  templates, sampling, stop handling, and decode state, but only for supported
  dense Llama-family models.
- `modelopt_nvfp4.rs` validates sharded ModelOpt checkpoints and exposes typed
  FP8/NVFP4 tensor components.
- `modelopt_nvfp4_cuda.rs` and the CUDA/CUTLASS source expose initial
  capability-gated dequantization and linear kernels, but no model architecture
  forward pass.

The installed checkpoint is labelled Qwen 3.6 but declares
`Qwen3_5MoeForConditionalGeneration`, `model_type=qwen3_5_moe`, and
`text_config.model_type=qwen3_5_moe_text`. Its text tower has 40 layers:
30 linear-attention layers and 10 full-attention layers, 256 routed experts,
8 selected experts per token, shared experts, FP8 attention projections,
W4A16 NVFP4 expert/LM-head weights, and FP8 KV-cache metadata. It also includes
vision configuration, but the requested OpenAI path is text generation.

GitHub issue #228 tracks broad architecture parity. This change is the focused,
implementable slice for the Qwen 3.5-compatible hybrid-MoE ModelOpt contract.

## Goals / Non-Goals

**Goals:**

- Produce correct text generation for the installed checkpoint through the
  existing accelerator and OpenAI streaming paths.
- Support other checkpoints whose architecture descriptor and tensor contract
  match a registered Qwen 3.5-compatible profile.
- Compose FP8 and NVFP4 operators without eagerly densifying the full model.
- Preserve CPU correctness fallback for small fixtures and native acceleration
  for production-sized checkpoints.
- Keep architecture recognition explicit and fail closed for unknown variants.

**Non-Goals:**

- Vision/image input or execution of the vision tower.
- Generic support for every Qwen, MoE, or ModelOpt checkpoint.
- MTP/speculative decoding in the first implementation.
- Unbounded CPU execution of the 35B production checkpoint.
- Replacing the existing Llama, ONNX, or mock backends.

## Decisions

### 1. Separate architecture semantics from weight representation

Add an architecture registry that maps normalized model metadata to an
`ArchitectureDescriptor` and constructs a backend implementing the existing
buffered/streaming generation interface. The first descriptor accepts the
Qwen 3.5 MoE text contract and validates:

- accepted `model_type` / architecture identifiers;
- hybrid layer types and required configuration fields;
- embedding, normalization, attention, MoE, and LM-head tensor-name patterns;
- supported quantization assignment per operator;
- excluded unsupported components such as MTP and vision.

This is preferred over treating all NVFP4 checkpoints as compatible: NVFP4
describes weight encoding, not the forward graph.

### 2. Implement a text-only Qwen 3.5 hybrid decoder

The runtime owns:

- token embeddings and final normalization;
- full-attention layers with causal masking, grouped KV heads, partial rotary
  dimensions, and KV-cache state;
- linear-attention layers with their convolution/recurrent state and gated
  output projections;
- sparse MoE router logits, deterministic top-k selection, routing-weight
  normalization, routed experts, shared expert, and aggregation;
- LM-head logits and reuse of existing sampling/streaming logic.

The exact equations and tensor transforms must be validated against a trusted
reference implementation and fixture outputs before the production checkpoint
is enabled.

### 3. Use operator-level mixed precision

Introduce quantized linear dispatch behind one operator contract:

- BF16/F16/F32 passthrough tensors use Candle dense operators;
- FP8 ModelOpt tensors use validated block scales and input scales;
- W4A16 NVFP4 tensors use existing packed weights, block scales, tensor scales,
  and optional activation scales;
- native CUDA/CUTLASS kernels are selected only when capabilities match;
- fallback dequantization is limited to the active operator/layer window.

This avoids constructing one dense copy of all 256 experts.

### 4. Make memory profile part of execution, not only loading

`performance` may retain a bounded working set of frequently selected experts
and layers. `layer-wise-streaming` maps shards and pages only active layer and
selected expert tensors. Full-attention KV cache and linear-attention recurrent
state are accounted separately and may be offloaded according to the existing
memory-profile contract.

### 5. Stage correctness before optimization

Implementation proceeds in gates:

1. metadata/tensor-contract validation;
2. tiny unquantized or synthetic mixed-precision fixture;
3. deterministic single-token forward parity;
4. multi-token state and streaming parity;
5. per-operator FP8/NVFP4 fallback;
6. native CUDA kernels and memory-profile integration;
7. opt-in installed-checkpoint smoke.

The production route is not enabled until the installed-checkpoint probe
generates non-mock output within explicit memory limits.

### 6. Dependency strategy

First inspect Candle 0.10 and the `astorise/candle` fork for reusable Qwen 3.5,
linear-attention, MoE, FP8, and cache primitives. Missing primitives are added
to the fork only when they are generally reusable; Tachyon-specific model
assembly and ModelOpt tensor mapping remain in `core-host`.

## Risks / Trade-offs

- [The checkpoint architecture is newer than Candle support] → Start with a
  dependency spike and isolate reusable primitives in the Candle fork.
- [Hybrid linear-attention state is implemented incorrectly] → Require
  layer-level golden fixtures and multi-token reference parity.
- [256 experts cause excessive open mappings or host memory] → Lazy-map shards,
  cache bounded tensor headers, and load only top-k plus shared experts.
- [Fallback dequantization is too large for the installed model] → Restrict
  fallback to tests/small models and require native capability for the 35B
  production checkpoint.
- [Mixed FP8/NVFP4 scaling semantics differ across ModelOpt versions] → Validate
  producer version and quantized-layer metadata, and reject unknown contracts.
- [Vision tensors are accidentally required] → Build from the text config and
  text tensor namespace only; reject multimodal request content explicitly.
- [Model naming suggests Qwen 3.6 while config says 3.5] → Compatibility is
  determined from normalized metadata and tensors, never the directory name.

## Migration Plan

1. Land architecture and operator fixtures without changing deployed routes.
2. Enable the new backend under `ai-inference`; keep native CUDA separately
   feature/capability gated.
3. Run the opt-in installed-checkpoint probe on the HomeLab node.
4. Add the model's dynamic binding to `/v1/chat/completions`.
5. Verify buffered and streaming OpenAI responses through
   `https://ai.tachyon-mesh.wsl/v1`.

Rollback removes the route binding or disables the architecture descriptor;
other model backends remain unchanged.

## Open Questions

- Which trusted reference implementation and fixture exporter will define
  golden intermediate tensors for Qwen 3.5 linear attention?
- Does the installed GPU expose native FP4 capability required for acceptable
  35B latency, or is a different optimized kernel path required?
- Which ModelOpt producer versions beyond 0.44.0 should be accepted initially?
