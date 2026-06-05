## Why

The current Candle-backed model binding path still returns `MOCK_LLM_RESPONSE` for non-NVFP4 aliases, so Tachyon can prove batching and routing but cannot yet execute a real local LLM through Candle. This change establishes the runtime contract needed to load supported Candle text-generation checkpoints and return generated model output without falling back to mock inference.

## What Changes

- Add a real Candle LLM runtime path for supported local text-generation model directories.
- Classify model bindings as explicit mock, ModelOpt/NVFP4 component sets, ONNX/WASI-NN, or supported Candle LLM backends instead of treating every non-NVFP4 binding as mock.
- Load tokenizer, config, and safetensors weights for the supported Candle LLM family selected for the first implementation.
- Parse prompt requests from the existing inference input bytes and return UTF-8 generated text bytes.
- Enforce prompt length, max-new-token, batch size, and deterministic default generation limits.
- Return typed unsupported-model or load errors when a binding is not a supported Candle LLM, rather than returning `MOCK_LLM_RESPONSE`.
- Keep `MOCK_LLM_RESPONSE` available only for explicit mock/test paths.
- Preserve existing Candle ONNX/WASI-NN behavior and the existing ModelOpt/NVFP4 unsupported-execution boundary.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `ai-inference`: Adds a real Candle LLM runtime contract for supported text-generation bindings and forbids non-mock aliases from falling back to mock output.
- `core-host`: Keeps new Candle LLM dependencies and validation paths behind the existing `ai-inference` feature.

## Impact

- Affected code: `core-host/src/ai_inference.rs`, a new Candle LLM runtime module under `core-host/src/ai_inference/`, AI inference tests, model-binding validation, and documentation.
- Affected dependencies: optional `candle-transformers` and tokenizer support under `core-host --features ai-inference`; no dependency is added to default builds.
- Runtime behavior changes for configured non-NVFP4 model bindings: supported Candle LLM directories generate real text, while unsupported directories fail with actionable errors.
- CI must validate the optional AI inference build and a deterministic small local LLM fixture without downloading external model artifacts.
