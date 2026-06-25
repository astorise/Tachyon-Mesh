## 1. Architecture Registry

- [x] 1.1 Add normalized HF/GGUF architecture identifiers and explicit format/execution capability metadata
- [x] 1.2 Refactor single-device model dispatch behind a family-neutral forward/reset boundary without changing Llama behavior
- [x] 1.3 Add detection and fail-closed tests for aliases, unsupported variants, formats, and parallel modes

## 2. Qwen Dense Runtime

- [x] 2.1 Integrate Qwen2 and Qwen3 safetensors config parsing, weight loading, EOS/context limits, and autoregressive state
- [x] 2.2 Add deterministic Qwen2/Qwen3 fixtures covering reference logits, multi-token generation, request limits, and streaming parity
- [x] 2.3 Integrate verified Qwen GGUF loaders or add architecture-specific unsupported-format diagnostics where loader coverage is incomplete

## 3. Gemma Runtime

- [x] 3.1 Integrate Gemma2 safetensors config parsing, weight loading, EOS/context limits, and autoregressive state
- [x] 3.2 Reject unsupported Gemma multimodal variants before loading weights
- [x] 3.3 Add deterministic Gemma2 fixtures covering multi-token generation, cache reset, and streaming parity
- [x] 3.4 Integrate verified Gemma GGUF loaders or add architecture-specific unsupported-format diagnostics where loader coverage is incomplete
- [x] 3.5 Fix Gemma3 non-contiguous KV-cache append in the pinned `astorise/candle` fork and enable the text-only backend
- [x] 3.6 Add deterministic Gemma3 reference-logit, request-limit, multi-token, and streaming fixtures after the fork fix is pinned

## 4. Phi and DeepSeek Follow-up Families

- [x] 4.1 Map verified Phi3/Phi4 identifiers to Candle implementations and add fixture-backed safetensors execution
- [x] 4.2 Map verified DeepSeek V2/V3/R1 identifiers to Candle implementations and add fixture-backed safetensors execution
- [x] 4.3 Keep unverified family variants and unsupported parallel/quantized combinations behind actionable typed errors

## 5. Regression, Documentation, and Validation

- [x] 5.1 Verify Llama, Mixtral expert-parallel, Qwen 3.5 MoE, ModelOpt/NVFP4, ONNX, and mock dispatch regressions
- [x] 5.2 Document the architecture-by-format-by-execution-mode compatibility matrix and link GitHub issue #228
- [x] 5.3 Run formatting, targeted runtime tests, `cargo check -p core-host --features ai-inference`, and OpenSpec validation
