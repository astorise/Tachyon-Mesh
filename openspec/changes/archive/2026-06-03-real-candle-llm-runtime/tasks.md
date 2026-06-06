## 1. Runtime Boundaries

- [x] 1.1 Add optional Candle LLM dependencies under the existing `ai-inference` feature without changing default builds.
- [x] 1.2 Add a `candle_llm_runtime` module with a narrow load/generate interface and typed load/execution errors.
- [x] 1.3 Add backend classification for explicit mock, ModelOpt/NVFP4, supported Candle LLM, and unsupported bindings.
- [x] 1.4 Ensure unsupported non-mock bindings fail during initialization instead of registering a mock backend.

## 2. Model Loading

- [x] 2.1 Select and document the first small Candle-supported causal LM fixture family.
- [x] 2.2 Implement local tokenizer loading for the selected fixture family.
- [x] 2.3 Implement config parsing and architecture validation for the selected fixture family.
- [x] 2.4 Implement safetensors weight loading into Candle tensors for the selected fixture family.
- [x] 2.5 Add typed errors for missing tokenizer, missing config, missing weights, unsupported architecture, and invalid tensor shapes.

## 3. Generation

- [x] 3.1 Parse plain UTF-8 prompts from the first `U8` input tensor.
- [x] 3.2 Add optional JSON generation request parsing with `prompt`, `max_new_tokens`, `temperature`, and `seed`.
- [x] 3.3 Enforce prompt byte/token, max-new-token, and batch-size limits before generation.
- [x] 3.4 Implement deterministic default generation through Candle and return UTF-8 bytes.
- [x] 3.5 Return typed validation or execution errors without falling back to `MOCK_LLM_RESPONSE`.

## 4. Mock and Existing Backend Preservation

- [x] 4.1 Keep `MOCK_LLM_RESPONSE` only in explicit mock/test registries and fixtures.
- [x] 4.2 Update tests that currently rely on implicit mock fallback to declare explicit mock bindings.
- [x] 4.3 Preserve the existing candle-onnx WASI-NN graph-loading path.
- [x] 4.4 Preserve the existing ModelOpt/NVFP4 classification and unsupported-execution error behavior.

## 5. Tests and CI

- [x] 5.1 Add a deterministic real Candle LLM fixture test that runs without network access.
- [x] 5.2 Add unsupported-binding tests proving non-mock safetensors directories do not return mock output.
- [x] 5.3 Add invalid tokenizer/config/weights tests with alias and path in the error message.
- [x] 5.4 Add prompt/generation limit tests for plain text and JSON requests.
- [x] 5.5 Update CI to run the real Candle fixture under `cargo test -p core-host --features ai-inference` or an equivalent focused check.

## 6. Documentation

- [x] 6.1 Document supported Candle LLM fixture/model requirements and unsupported architectures.
- [x] 6.2 Document the prompt input formats, generation defaults, and limits.
- [x] 6.3 Document that NVFP4 remains a separate loader/kernel capability and is not made text-generatable by this change.
