## Context

`candle_llm_runtime.rs` currently combines format detection, Llama-specific
configuration parsing, model construction, KV-cache ownership, and decode
dispatch. Safetensors loading rejects every `model_type` except `llama`; GGUF
loading rejects every `general.architecture` except `llama`; the parallel
engines are also Llama-specific. The pinned Candle fork already exposes Qwen2,
Qwen3, Gemma2, Gemma3, Phi3, and DeepSeek2 model implementations, but their
configuration, mutable cache ownership, EOS conventions, and forward signatures
are not integrated into Tachyon.

The runtime must preserve its existing tokenizer, chat template, sampling,
constrained decoding, stop handling, streaming, batching, route sealing, and
generation limits. Architecture expansion must therefore happen below the
request layer and above Candle's family-specific model implementations.

## Goals / Non-Goals

**Goals:**

- Select a backend from normalized HF or GGUF architecture metadata.
- Execute Qwen2/Qwen3 dense and Gemma2/Gemma3 safetensors checkpoints first.
- Provide one extension contract for Phi and DeepSeek follow-up integrations.
- Keep generation behavior and errors consistent across architecture families.
- Make execution-mode support explicit and testable per architecture.
- Fail before weight loading when a format, variant, or parallel mode is not
  supported.

**Non-Goals:**

- Claim support for every model sharing a broad family name.
- Add multimodal vision towers or image inputs.
- Fold the specialized Qwen 3.5 MoE ModelOpt/NVFP4 runtime into this loader.
- Implement generic tensor/pipeline/expert sharding for every new family in the
  first tranche.
- Download production checkpoints in CI.

## Decisions

### 1. Introduce a normalized architecture descriptor

Add a small `ModelArchitecture` enum and a descriptor produced by probing
`config.json` or GGUF metadata. The descriptor contains the normalized family,
weight format, context limit, and supported execution modes. Model names and
directory names never select a backend.

Accepted HF identifiers are explicit aliases, for example `qwen2`, `qwen3`,
`gemma2`, and `gemma3_text`. Composite multimodal configs may expose a nested
text config, but they are accepted only when the loader can isolate a supported
text-only tensor namespace. Unknown aliases remain unsupported.

This is preferred over scattered string comparisons because all format and
execution compatibility decisions become auditable in one place.

### 2. Use a family-neutral autoregressive backend interface

Refactor `LoadedModel` so single-device safetensors backends implement a common
forward/reset contract. Each backend owns the mutable state required by its
Candle model. Llama may retain its external per-request cache; Qwen, Gemma, Phi,
and DeepSeek models whose Candle implementation owns mutable caches are guarded
by a mutex and reset at the start of each sequence.

The shared decode loop continues to own tokenization, sampling, constraints,
stop matching, and streaming. It receives logits through a closure or trait
object and does not contain family-specific branches.

Using one giant enum match in every decode step was considered, but it would
spread architecture knowledge through request handling and make later families
costlier to add.

### 3. Deliver support in verified family gates

The first implementation gate covers dense Qwen2/Qwen3 and Gemma2/Gemma3
safetensors on the existing single-device CPU path, and CUDA when the same
Candle model loads on a CUDA device. Phi3/Phi4 and DeepSeek V2/V3/R1 are added
only after fixture parity establishes the exact supported identifiers and
tensor contracts.

A family is advertised as supported only after config parsing, weight loading,
single-token logits, multi-token cache reuse, buffered generation, and streaming
generation pass deterministic tests. Presence of a Candle module alone is not
enough.

### 4. Keep GGUF dispatch separate from HF dispatch

GGUF support is registered per quantized loader. Qwen2/Qwen3 and Gemma3 can be
enabled where the pinned Candle fork exposes a matching quantized model;
families without a matching loader return an actionable error naming the
recognized architecture and unsupported format. HF support does not
automatically imply GGUF support.

### 5. Declare parallel compatibility per backend

The existing tensor-, pipeline-, and expert-parallel engines remain
Llama/Mixtral-specific. A non-single deployment for a new family is rejected
before weights are mapped unless that backend explicitly implements the
requested mode. This prevents accidental use of Llama tensor names or cache
semantics.

Future family-specific parallel adapters can register capabilities without
changing configuration or request APIs.

### 6. Use tiny local fixtures and reference logits

Tests create or check in minimal family-specific configs, tokenizers, and
safetensors with deterministic weights. Golden logits and generated token IDs
are compared against the corresponding Candle model directly. Optional ignored
smoke tests may use operator-provided model directories, but CI performs no
network downloads.

## Risks / Trade-offs

- [Candle family APIs have incompatible cache ownership] → Hide them behind the
  backend forward/reset contract and serialize stateful models per loaded alias.
- [Broad aliases accept incompatible variants] → Maintain explicit accepted
  identifiers and validate required config fields and tensor names before load.
- [Mutex-backed models reduce concurrency] → Preserve scheduler batching and
  add per-request cache cloning later only where the Candle API supports it.
- [GGUF parity lags safetensors] → Report format-specific support instead of
  overpromising family support.
- [Phi4 or DeepSeek V3 differs from existing Candle modules] → Treat each alias
  as unsupported until a fixture proves compatibility; use the Candle fork for
  reusable upstreamable primitives.
- [New backends accidentally enter Llama parallel engines] → Capability-check
  execution mode before any topology validation or weight mapping.

## Migration Plan

1. Land architecture probing and backend dispatch with Llama behavior covered by
   regression tests.
2. Enable Qwen2/Qwen3 dense fixtures, then Gemma2/Gemma3 fixtures.
3. Add format-specific GGUF loaders where verified.
4. Add Phi and DeepSeek adapters in separate commits using the same contract.
5. Update model compatibility documentation and run opt-in real-checkpoint
   smoke tests.

Rollback removes individual backend registrations; Llama, Mixtral, Qwen 3.5
MoE, NVFP4, and ONNX paths remain independently selectable.

## Open Questions

- Which exact Phi4 and DeepSeek V3/R1 config aliases in the pinned Candle fork
  are forward-compatible with its current Phi3 and DeepSeek2 implementations?
- Should stateful backends initially serialize generation per alias or should
  Tachyon construct a bounded pool of model/cache instances?
- Which non-Llama GGUF loaders in the fork have sufficient tokenizer and
  long-context coverage to advertise in the first release?
