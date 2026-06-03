## ADDED Requirements

### Requirement: AI inference bindings MUST classify ModelOpt/NVFP4 directories without mock execution
The AI inference runtime SHALL load supported ModelOpt/NVFP4 model bindings as typed component sets and SHALL NOT return mock inference output for those aliases.

#### Scenario: Detected NVFP4 alias refuses execution until backend is configured
- **WHEN** a preloaded model alias is classified as ModelOpt/NVFP4
- **AND** a guest or host caller submits an inference request for that alias
- **THEN** the runtime returns an actionable unsupported-execution error
- **AND** the response is not `MOCK_LLM_RESPONSE`

#### Scenario: Existing ONNX guest path remains available
- **WHEN** a legacy guest loads an ONNX model through WASI-NN
- **THEN** the host continues to use the candle-onnx backend
- **AND** ModelOpt/NVFP4 loading does not change the ONNX graph encoding contract

### Requirement: Unsupported quantized model bindings MUST fail with actionable errors
The AI inference runtime SHALL reject unsupported quantized model bindings with typed errors before returning mock inference output.

#### Scenario: Unsupported ModelOpt layout is configured
- **WHEN** a model binding points at a ModelOpt/NVFP4 checkpoint whose tensor names, scale layout, or architecture are not supported
- **THEN** model initialization fails with a typed error containing the model alias, model path, and unsupported layout detail
- **AND** inference for that alias is not registered

#### Scenario: Non-NVFP4 model remains outside the NVFP4 loader
- **WHEN** a model binding points at a safetensors directory without NVFP4 metadata or NVFP4 scale tensors
- **THEN** the ModelOpt/NVFP4 loader does not claim the binding
- **AND** the host either routes the model to another supported backend or returns an unsupported-model error
