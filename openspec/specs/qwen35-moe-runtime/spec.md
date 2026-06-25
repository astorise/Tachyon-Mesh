# qwen35-moe-runtime Specification

## Purpose
TBD - created by archiving change add-qwen35-moe-nvfp4-runtime. Update Purpose after archive.
## Requirements
### Requirement: Runtime MUST recognize compatible Qwen 3.5 MoE text architectures

The runtime SHALL recognize Qwen 3.5-compatible MoE text checkpoints from
normalized model metadata and tensor contracts, not from directory names.

#### Scenario: Compatible architecture is selected

- **WHEN** a checkpoint declares a supported Qwen 3.5 MoE architecture and text
  configuration
- **AND** its required tensor names and shapes match the registered descriptor
- **THEN** the runtime selects the Qwen 3.5 MoE text backend

#### Scenario: NVFP4 alone is insufficient

- **WHEN** a checkpoint uses ModelOpt NVFP4 but its architecture or tensor
  contract does not match a registered descriptor
- **THEN** the runtime rejects it with an actionable architecture-compatibility
  error

#### Scenario: Directory marketing name differs from metadata

- **WHEN** the model directory or registry alias names a different Qwen release
- **THEN** backend selection SHALL use `config.json`, text configuration,
  quantization metadata, and indexed tensors

### Requirement: Runtime MUST execute hybrid linear and full attention

The Qwen 3.5 MoE backend SHALL execute the declared ordered sequence of
linear-attention and full-attention decoder layers with the required gated
projections and position encoding.

#### Scenario: Hybrid layer schedule is preserved

- **WHEN** the configuration declares a mixture of linear-attention and
  full-attention layer types
- **THEN** each layer executes the operator type declared at its index
- **AND** hidden states flow through the layers in declaration order

#### Scenario: Full attention maintains KV state

- **WHEN** autoregressive decode crosses a full-attention layer
- **THEN** the backend updates and reuses causal key/value cache state for that
  layer

#### Scenario: Linear attention maintains recurrent state

- **WHEN** autoregressive decode crosses a linear-attention layer
- **THEN** the backend updates and reuses the convolutional or recurrent state
  required by that layer

### Requirement: Runtime MUST execute sparse routed and shared experts

The backend SHALL compute router logits, select the configured number of
experts per token, normalize routing weights according to the architecture,
execute only selected routed experts plus the shared expert, and aggregate
their outputs.

#### Scenario: Top-k routing is deterministic

- **WHEN** router logits are equal across repeated deterministic runs
- **THEN** the same expert indices and routing weights are selected

#### Scenario: Only selected experts execute

- **WHEN** a layer declares many experts and a smaller `num_experts_per_tok`
- **THEN** the forward pass executes only the selected routed experts for each
  token plus configured shared experts

#### Scenario: Invalid expert tensors are rejected

- **WHEN** an expert group is missing a required gate, up, down, scale, or
  activation component
- **THEN** model loading fails with the layer index, expert index, and missing
  component

### Requirement: Runtime MUST provide bounded text generation

The backend SHALL reuse Tachyon's chat-template, tokenization, sampling, stop,
buffered generation, and incremental streaming contracts.

#### Scenario: Buffered generation produces real text

- **WHEN** a valid prompt targets a compatible checkpoint
- **THEN** the backend returns generated UTF-8 text that is not mock output

#### Scenario: Streaming reconstructs buffered output

- **WHEN** equivalent deterministic requests are run buffered and streamed
- **THEN** concatenated streamed fragments equal the buffered generated text

#### Scenario: Vision input remains unsupported

- **WHEN** a request supplies image content to the text-only backend
- **THEN** the runtime rejects it with an explicit unsupported-modality error

### Requirement: Compatibility profile MUST be extensible and fail closed

The architecture registry SHALL permit additional checkpoint variants to reuse
the backend only when a versioned compatibility profile validates their model
metadata, operator semantics, quantization assignments, and tensor contract.

#### Scenario: Compatible sibling checkpoint is accepted

- **WHEN** another checkpoint matches a registered compatibility profile despite
  different layer or expert counts
- **THEN** the runtime parameterizes the same backend from its configuration

#### Scenario: Semantic variant is rejected

- **WHEN** a checkpoint introduces an unknown layer type, router rule, position
  encoding, or quantization assignment
- **THEN** the runtime rejects it until a new compatibility profile is
  implemented and tested

