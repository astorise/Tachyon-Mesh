## MODIFIED Requirements

### Requirement: AI inference bindings MUST classify ModelOpt/NVFP4 directories without mock execution

The AI inference runtime SHALL load supported ModelOpt/NVFP4 model bindings as
typed component sets and SHALL NOT return mock inference output for those
aliases. When a registered architecture backend matches the checkpoint, the
runtime SHALL execute real inference; otherwise it SHALL return an actionable
unsupported-architecture error.

#### Scenario: Detected NVFP4 alias executes with a supported architecture

- **WHEN** a preloaded model alias is classified as ModelOpt/NVFP4
- **AND** a registered architecture backend validates its metadata and tensors
- **THEN** inference executes through that architecture backend
- **AND** the response is not `MOCK_LLM_RESPONSE`

#### Scenario: Detected NVFP4 alias refuses an unsupported architecture

- **WHEN** a preloaded model alias is classified as ModelOpt/NVFP4
- **AND** no registered architecture backend accepts it
- **THEN** the runtime returns an actionable unsupported-architecture error
- **AND** the response is not `MOCK_LLM_RESPONSE`

#### Scenario: Existing ONNX guest path remains available

- **WHEN** a legacy guest loads an ONNX model through WASI-NN
- **THEN** the host continues to use the candle-onnx backend
- **AND** ModelOpt/NVFP4 loading does not change the ONNX graph encoding contract

### Requirement: Existing ONNX and NVFP4 boundaries MUST remain unchanged

Adding a new ModelOpt/NVFP4 architecture runtime SHALL NOT change legacy Candle
ONNX/WASI-NN graph loading. NVFP4 checkpoints SHALL execute only when an
explicit architecture backend validates their metadata and tensor contract;
all other NVFP4 checkpoints SHALL preserve the non-mock unsupported boundary.

#### Scenario: Legacy ONNX guest still uses candle-onnx

- **WHEN** a legacy guest loads an ONNX model through WASI-NN
- **THEN** the host continues to use the candle-onnx backend
- **AND** architecture backend selection does not change the ONNX graph
  encoding contract

#### Scenario: Supported ModelOpt/NVFP4 alias generates text

- **WHEN** a preloaded ModelOpt/NVFP4 alias matches a registered text-generation
  architecture backend
- **THEN** buffered and streaming inference execute real model generation
- **AND** the response is not `MOCK_LLM_RESPONSE`

#### Scenario: Unsupported ModelOpt/NVFP4 alias remains non-mock

- **WHEN** a preloaded model alias is classified as ModelOpt/NVFP4
- **AND** no architecture backend is configured for that alias
- **THEN** inference returns an actionable unsupported-execution error
- **AND** the response is not `MOCK_LLM_RESPONSE`

### Requirement: The runtime streams decoded fragments incrementally

The runtime SHALL provide a streaming generation path that emits each newly
decoded text fragment as it is produced, such that the concatenation of all
fragments equals the buffered generation output for the same request. While
streaming with stop sequences, the runtime SHALL hold back the trailing text
that could begin a stop match until a further token confirms it is safe to emit.

#### Scenario: Streamed fragments reconstruct the buffered output

- **WHEN** the same request is run buffered and streamed
- **THEN** the streamed path emits one or more fragments
- **AND** their concatenation equals the buffered output byte-for-byte

#### Scenario: Non-generative backends fall back to a single fragment

- **WHEN** a streaming request targets a backend that cannot decode
  incrementally, such as an explicit mock backend
- **THEN** the runtime emits the entire output as one fragment

#### Scenario: Supported NVFP4 architecture streams tokens

- **WHEN** a streaming request targets a ModelOpt/NVFP4 checkpoint with a
  registered autoregressive architecture backend
- **THEN** decoded text fragments are emitted incrementally as tokens are
  generated
