## MODIFIED Requirements

### Requirement: PagedAttention MUST require an explicit block-table runtime path
When a model deployment sets `hardware_strategy.paged_attention: true`, the runtime SHALL NOT silently fall back to the existing contiguous per-request KV cache. Tachyon SHALL enable this mode only for architectures and devices where its core-host runtime owns a block allocator, a per-sequence block table, and a Candle paged flash-attn call using `flash_attn_varlen_paged_windowed` or a compatible successor API; every other architecture/device combination SHALL keep failing closed with a typed error.

#### Scenario: PagedAttention request is rejected before Tachyon block tables are wired
- **GIVEN** a model binding sets `hardware_strategy.paged_attention: true`
- **AND** the runtime build does not yet have the block allocator/block-table integration for that binding's architecture
- **WHEN** the Candle LLM runtime loads the binding
- **THEN** loading fails with a typed `UnsupportedModel` error naming the missing block allocator and block-table integration
- **AND** the runtime does not execute the contiguous KV-cache path as a fallback

#### Scenario: PagedAttention is rejected on a non-Llama architecture
- **GIVEN** a model binding sets `hardware_strategy.paged_attention: true`
- **AND** the binding's architecture is not Llama
- **WHEN** the Candle LLM runtime loads the binding
- **THEN** loading fails with a typed `UnsupportedModel` error naming the unsupported architecture
- **AND** the runtime does not execute the contiguous KV-cache path as a fallback

#### Scenario: PagedAttention is rejected on a non-CUDA device
- **GIVEN** a Llama model binding sets `hardware_strategy.paged_attention: true`
- **AND** the requested device is not a CUDA device
- **WHEN** the Candle LLM runtime loads the binding
- **THEN** loading fails with a typed `UnsupportedModel` error naming the CUDA-only requirement
- **AND** the runtime does not execute the contiguous KV-cache path as a fallback

#### Scenario: PagedAttention is enabled for a Llama binding on CUDA once block-paged KV integration is available
- **GIVEN** the runtime build owns a CUDA block pool, per-sequence block tables, and a paged K/V tensor layout compatible with Candle's paged flash-attn API
- **AND** a Llama model binding requesting a CUDA device sets `hardware_strategy.paged_attention: true`
- **WHEN** the Candle LLM runtime loads the binding
- **THEN** the load succeeds and decode uses the block-paged attention path
- **AND** the model's weights and KV cache load in BF16 rather than the contiguous path's F32, because the paged flash-attention kernel only supports F16/BF16
- **AND** sequence admission and eviction operate at block granularity rather than reallocating a contiguous KV cache per request
- **AND** generation output is a real decode over the loaded weights (not a mock), consistent (repeated identical greedy requests against the same loaded binding produce identical output) though not necessarily bit-identical to the F32 contiguous path given the BF16 precision difference

#### Scenario: PagedAttention KV pool sizing fails closed when the budget can't fit one full sequence
- **GIVEN** a Llama model binding on a CUDA device sets `hardware_strategy.paged_attention: true`
- **AND** the device's free VRAM (after the model's weights are loaded) cannot fit enough paged KV blocks to hold one sequence of the checkpoint's `max_position_embeddings` length
- **WHEN** the Candle LLM runtime loads the binding
- **THEN** loading fails with a typed `UnsupportedModel` error naming the sizing shortfall
- **AND** no paged KV block pool or per-layer tensors are left allocated
