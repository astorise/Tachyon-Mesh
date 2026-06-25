## ADDED Requirements

### Requirement: Native Candle inference MUST dispatch supported non-Llama architectures

The native Candle text-generation path SHALL dispatch supported non-Llama
safetensors and GGUF checkpoints through registered architecture-specific
backends while preserving the existing sealed-alias authorization, request
schema, scheduler, sampling, constrained decoding, stop, buffered, and streaming
behavior. Existing Llama, Mixtral, Qwen 3.5 MoE, ModelOpt/NVFP4, ONNX, and mock
backend boundaries SHALL remain unchanged.

#### Scenario: Supported non-Llama alias uses native Candle execution

- **WHEN** a route-authorized model alias resolves to a checkpoint whose
  architecture and format have a registered backend
- **THEN** the native Candle runtime executes that backend
- **AND** the response is not `MOCK_LLM_RESPONSE`

#### Scenario: Unsupported architecture remains actionable and non-mock

- **WHEN** a route-authorized alias resolves to an architecture or format
  combination without a registered backend
- **THEN** the runtime returns an actionable typed unsupported-model error
- **AND** does not return mock inference output

#### Scenario: Existing Llama behavior is unchanged

- **WHEN** a Llama safetensors or supported Llama GGUF checkpoint is loaded
- **THEN** its existing single-device generation path and outputs remain
  compatible with the pre-registry behavior

#### Scenario: Existing specialized runtimes remain independently dispatched

- **WHEN** a checkpoint matches the Qwen 3.5 MoE ModelOpt/NVFP4 contract, the
  Mixtral expert-parallel contract, or the legacy ONNX contract
- **THEN** the existing specialized dispatcher handles it
- **AND** the generic architecture registry does not reinterpret it as a dense
  Qwen, Gemma, Phi, or DeepSeek checkpoint
