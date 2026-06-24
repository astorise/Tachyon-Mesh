## ADDED Requirements

### Requirement: Layer-wise sparse-MoE execution MUST page only active experts

For compatible Qwen 3.5 MoE checkpoints, layer-wise streaming SHALL map and
transfer only the active layer, selected routed experts, shared expert, and
required attention state.

#### Scenario: Inactive experts remain off accelerator

- **WHEN** a token selects a subset of experts in layer-wise mode
- **THEN** non-selected experts for that layer remain outside accelerator
  memory

#### Scenario: Sharded experts resolve through the tensor index

- **WHEN** selected experts span multiple safetensors shards
- **THEN** each expert tensor is resolved by name through
  `model.safetensors.index.json`

### Requirement: Hybrid attention state MUST respect memory profiles

The memory-profile implementation SHALL account separately for full-attention
KV cache and linear-attention recurrent state.

#### Scenario: Full-attention cache is paged

- **WHEN** layer-wise decode leaves a full-attention layer
- **THEN** its KV cache is retained or offloaded according to the configured
  memory profile without losing autoregressive state

#### Scenario: Linear-attention state is paged

- **WHEN** layer-wise decode leaves a linear-attention layer
- **THEN** its convolutional or recurrent state is retained or offloaded
  according to the configured memory profile
