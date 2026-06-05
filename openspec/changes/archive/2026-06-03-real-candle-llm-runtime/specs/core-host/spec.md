## ADDED Requirements

### Requirement: Candle LLM dependencies MUST remain feature-gated
The `core-host` crate SHALL keep tokenizer and Candle text-generation dependencies optional under the existing `ai-inference` feature and SHALL keep the default host build free of those dependencies.

#### Scenario: Default host build excludes Candle LLM runtime
- **WHEN** a developer builds `core-host` without `--features ai-inference`
- **THEN** tokenizer and Candle LLM runtime dependencies are not linked
- **AND** the default release and container workflows remain unchanged

#### Scenario: AI inference build includes Candle LLM runtime
- **WHEN** a developer builds `core-host` with `--features ai-inference`
- **THEN** the Candle LLM runtime module, tokenizer support, and selected Candle text-generation dependency are compiled
- **AND** existing ONNX/WASI-NN AI inference support remains available

#### Scenario: AI guest runs without ai-inference feature
- **WHEN** `core-host` is built without `--features ai-inference`
- **AND** an AI guest or route requires a model binding
- **THEN** execution fails gracefully with an error naming the missing `ai-inference` feature
