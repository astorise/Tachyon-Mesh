## ADDED Requirements

### Requirement: The runtime MUST select text-generation backends from normalized architecture metadata

The Candle text-generation runtime SHALL inspect Hugging Face `config.json`
metadata or GGUF `general.architecture`, normalize recognized identifiers to an
explicit architecture descriptor, and construct only the backend registered for
that architecture and weight format. Directory names and model aliases SHALL
NOT determine architecture compatibility.

#### Scenario: Hugging Face architecture selects a registered backend

- **WHEN** a safetensors checkpoint declares a supported Qwen, Gemma, Phi, or
  DeepSeek text `model_type`
- **THEN** the runtime selects the corresponding registered family backend
- **AND** validates the family config before loading weights

#### Scenario: GGUF architecture is recognized but has no registered loader

- **WHEN** a GGUF checkpoint declares a known architecture without a verified
  quantized loader
- **THEN** the runtime returns a typed unsupported-model error naming both the
  architecture and GGUF format
- **AND** does not attempt to load it with the Llama GGUF loader

#### Scenario: Directory naming cannot override metadata

- **WHEN** a model directory or alias contains `qwen`, `gemma`, `phi`, or
  `deepseek` but its embedded architecture metadata is unsupported
- **THEN** the runtime rejects the checkpoint based on the embedded metadata

### Requirement: Qwen and Gemma dense checkpoints MUST execute native generation

The runtime SHALL support single-device native text generation for verified
Qwen2/Qwen3 dense and Gemma2/Gemma3 safetensors checkpoints using their
family-specific Candle models. Supported backends SHALL produce logits and
generated text through the same request contract as the existing Llama backend.

#### Scenario: Qwen dense checkpoint generates multiple tokens

- **WHEN** a valid Qwen2 or Qwen3 dense safetensors checkpoint is loaded
- **AND** a generation request asks for more than one token
- **THEN** the runtime performs prefill and autoregressive decode with persistent
  family-appropriate cache state
- **AND** returns non-mock generated text

#### Scenario: Gemma checkpoint generates multiple tokens

- **WHEN** a valid Gemma2 or text-only Gemma3 safetensors checkpoint is loaded
- **AND** a generation request asks for more than one token
- **THEN** the runtime performs prefill and autoregressive decode with persistent
  family-appropriate cache state
- **AND** returns non-mock generated text

#### Scenario: Multimodal Gemma variant is not silently treated as text-only

- **WHEN** a Gemma checkpoint requires a vision tower or multimodal inputs that
  the registered backend does not implement
- **THEN** the runtime rejects it with an actionable unsupported-variant error

### Requirement: Architecture backends MUST preserve shared generation semantics

Every registered architecture backend SHALL use the host's existing prompt
limits, tokenizer and chat-template rendering, sampling, constrained decoding,
EOS handling, stop sequences, buffered output, and incremental streaming
contracts.

#### Scenario: Buffered and streamed output remain equivalent

- **WHEN** the same deterministic request is executed against a registered
  non-Llama backend in buffered and streaming modes
- **THEN** concatenating streamed fragments yields the buffered output
  byte-for-byte

#### Scenario: Family EOS tokens terminate decode

- **WHEN** a registered backend emits any configured EOS token for its family
- **THEN** generation stops without exposing the EOS token as user text

#### Scenario: Request limits apply to every architecture

- **WHEN** a request to a non-Llama backend exceeds host prompt or generation
  limits
- **THEN** the runtime rejects it through the same typed invalid-request
  boundary used by existing text-generation backends

### Requirement: Architecture and execution-mode compatibility MUST fail closed

Each architecture backend SHALL declare the weight formats and single, tensor,
pipeline, or expert execution modes it implements. The runtime SHALL reject an
unsupported combination before mapping model weights and SHALL NOT silently
downgrade or route it through a Llama-specific parallel engine.

#### Scenario: New architecture requests unsupported tensor parallelism

- **WHEN** a Qwen, Gemma, Phi, or DeepSeek binding requests tensor parallelism
- **AND** that family backend has not registered tensor-parallel support
- **THEN** model loading fails before weights are allocated
- **AND** the error names the architecture and requested execution mode

#### Scenario: Supported single-device mode remains available

- **WHEN** a verified architecture binding requests single-device execution
- **THEN** the registered backend loads without invoking Llama-specific topology
  or sharding code

### Requirement: Family support MUST be proven by deterministic fixtures

A family or format SHALL be advertised as supported only when repository tests
cover metadata detection, config validation, weight loading, reference logits,
multi-token state reuse, buffered generation, and streaming generation using
small local fixtures without downloading external checkpoints.

#### Scenario: CI validates an advertised family offline

- **WHEN** CI tests the optional `ai-inference` feature
- **THEN** every advertised architecture family executes its local fixture
- **AND** the tests require no network access or production-sized model artifact

#### Scenario: Unsupported variant remains explicit

- **WHEN** a family identifier is recognized but lacks a complete fixture-backed
  implementation
- **THEN** the runtime reports it as a recognized unsupported variant
- **AND** documentation does not list it as executable
