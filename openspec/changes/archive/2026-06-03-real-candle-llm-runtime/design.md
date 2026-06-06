## Context

Tachyon already has an optional `ai-inference` feature, Candle ONNX support for legacy WASI-NN guests, model bindings declared in the integrity config, batching queues, explicit test-only graph mocks, and the separate ModelOpt/NVFP4 loader primitives. The remaining gap is that `CandleBackendModel::execute` still performs a mock batch allocation and returns `MOCK_LLM_RESPONSE` for every non-NVFP4 binding.

This change is not another NVFP4 or Qwen-specific scope. It introduces a real Candle text-generation runtime for a supported local checkpoint family, while preserving the current ONNX path and the NVFP4 "classified but unsupported for architecture execution" boundary.

## Goals / Non-Goals

**Goals:**
- Add a supported Candle LLM backend that loads local tokenizer, config, and safetensors weights.
- Generate real UTF-8 text bytes for supported model bindings.
- Make mock inference explicit and test-scoped instead of the default for non-NVFP4 bindings.
- Bound generation work with deterministic defaults suitable for CI.
- Keep heavy LLM dependencies feature-gated under `ai-inference`.
- Preserve ONNX/WASI-NN and ModelOpt/NVFP4 behavior.

**Non-Goals:**
- Implement Qwen3.6, MoE routing, chat templates, tensor parallelism, KV-cache optimization, LoRA hot-swapping, or NVFP4 execution in this change.
- Download models during runtime or CI.
- Support arbitrary Hugging Face architectures on the first pass.
- Replace the WASI-NN ONNX backend.

## Decisions

### Add a distinct Candle LLM backend kind

`CandleBackendModelKind` will grow from `Mock | ModelOptNvfp4` to include a real text-generation variant, for example `TextGeneration(CandleLlmRuntime)`.

Binding classification order:
1. Explicit mock markers used by tests or fixtures.
2. ModelOpt/NVFP4 detection from the existing loader.
3. Supported Candle LLM directory detection.
4. Typed unsupported-model error.

The important behavior change is that step 4 fails; it does not become mock output.

### Prefer one small supported architecture first

The first implementation should wire one Candle-supported causal LM family with a small deterministic fixture that can live in the repository or be generated locally in tests. Candidate families are small GPT-style models already supported by `candle-transformers`.

This avoids pretending Tachyon supports every Hugging Face config layout. Additional architectures can be added later as separate backend adapters with their own tests.

### Keep the inference input contract byte-oriented

The current scheduler forwards `SharedInputTensor` bytes. The Candle LLM runtime will interpret the first `U8` input as either:
- plain UTF-8 prompt text, or
- a compact JSON generation request containing `prompt`, `max_new_tokens`, `temperature`, and `seed`.

Plain UTF-8 uses deterministic defaults. JSON requests are optional and bounded by the same limits. The output remains UTF-8 bytes so existing host/guest plumbing does not need a schema migration in this change.

### Deterministic generation by default

Default generation uses greedy decoding or a fixed seeded sampler, a small max-new-token cap, and explicit prompt/token limits. Batch size remains bounded by the existing scheduler configuration, and unsupported mixed request shapes fail with typed errors.

This makes CI stable and protects the host from unbounded prompt or decode work.

### Use feature-gated Candle dependencies

The implementation may add optional dependencies such as `candle-transformers` and `tokenizers`, included only in the existing `ai-inference` feature. Default `core-host` builds must not link tokenizer or LLM model code.

No `hf-hub` download behavior is required. Model paths are local integrity bindings.

### Preserve existing boundaries

The Candle ONNX path continues to use `candle-onnx` for legacy WASI-NN graph loading. ModelOpt/NVFP4 directories continue to be classified by the NVFP4 loader and return unsupported execution unless a future architecture/runtime change wires them into a complete LLM execution path.

`MOCK_LLM_RESPONSE` remains valid for explicit test registries and explicit mock model fixtures, but production non-mock model bindings must never reach it.

## Risks / Trade-offs

- Small real LLM fixtures can still increase repository or CI cost. Mitigation: use the smallest supported checkpoint fixture possible, or generate a deterministic tiny weight fixture in tests without network access.
- Candle architecture support varies by crate version. Mitigation: select one architecture already supported by the pinned Candle ecosystem and reject unknown configs explicitly.
- JSON generation requests could complicate guest behavior. Mitigation: keep plain UTF-8 prompt input as the default and make JSON request parsing additive.
- GPU execution may not be available on all hosts. Mitigation: start with CPU support and return typed errors for requested devices that are not wired.
- Removing default mock fallback can expose previously hidden misconfiguration. Mitigation: add actionable load errors containing alias, path, and unsupported reason.

## Migration Plan

1. Add feature-gated dependencies and a `candle_llm_runtime` module.
2. Add backend classification for explicit mock, NVFP4, supported LLM, and unsupported bindings.
3. Implement tokenizer/config/weight loading for the selected first architecture.
4. Implement bounded generation request parsing and UTF-8 output.
5. Update tests that expect `MOCK_LLM_RESPONSE` so they use explicit mock fixtures.
6. Add deterministic real Candle generation tests without external downloads.
7. Update docs and CI checks.

Rollback is to remove the LLM backend classification and dependencies while leaving explicit mock, ONNX, and NVFP4 behavior intact.

## Open Questions

- Which exact small Candle-supported causal LM fixture should be the first supported model family?
- Should JSON generation requests become part of a WIT/API contract in a later change, or remain an internal host convention for now?
