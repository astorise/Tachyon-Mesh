## MODIFIED Requirements

### Requirement: CUDA Graph and FlashInfer decode acceleration MUST be explicit and fail-closed
The AI inference build SHALL consume the pinned `astorise/candle` fork tag that
exposes `candle_core::CudaGraph` and the optional
`candle-flashinfer-kernels` crate for the downstream work proposed in
`huggingface/candle#3651`. Model deployments MAY declare
`hardware_strategy.cuda_graph_decode` and
`hardware_strategy.flashinfer_attention`. `cuda_graph_decode` SHALL be
enabled only for a Llama-family checkpoint on a CUDA device that also
declares `hardware_strategy.paged_attention: true` — the contiguous KV
cache's per-step reallocation is fundamentally incompatible with CUDA
graph replay, so `cuda_graph_decode` without `paged_attention` SHALL
continue to fail closed with a typed error naming that dependency, not
just naming the missing `CudaGraph` wiring. `flashinfer_attention`'s
requirement is unchanged from the prior modification in
`wire-flashinfer-decode-attention`.

#### Scenario: CUDA Graph decode request is rejected before capture is wired
- **GIVEN** a model binding sets `hardware_strategy.cuda_graph_decode: true`
- **AND** the runtime build does not yet have the capture/replay decode path wired for that binding's architecture, device, or build
- **WHEN** the Candle LLM runtime loads the binding
- **THEN** loading fails with a typed `UnsupportedModel` error naming
  `candle_core::CudaGraph`
- **AND** the runtime does not silently execute the uncaptured decode loop

#### Scenario: CUDA Graph decode without paged attention is rejected
- **GIVEN** a Llama model binding on CUDA sets `hardware_strategy.cuda_graph_decode: true`
- **AND** does not also set `hardware_strategy.paged_attention: true`
- **WHEN** the Candle LLM runtime loads the binding
- **THEN** loading fails with a typed `UnsupportedModel` error naming the `paged_attention` dependency
- **AND** the runtime does not silently fall back to capturing the contiguous KV cache path

#### Scenario: CUDA Graph decode is enabled for a paged-attention Llama binding on CUDA once capture is wired
- **GIVEN** the runtime build has the decode-position seam and capture/replay orchestration wired
- **AND** a Llama model binding requesting a CUDA device sets both `hardware_strategy.paged_attention: true` and `hardware_strategy.cuda_graph_decode: true`
- **WHEN** the Candle LLM runtime loads the binding and generates
- **THEN** the load succeeds, the first decode step runs a warm-up call followed by a `CudaGraph` capture, and every subsequent decode step replays that captured graph after updating the input-token, position, and paged block-table/seqlens buffers in place
- **AND** the block-table/seqlens tensors are sized to their full maximum width (`min_blocks`) from the first decode step, so no recapture occurs within a single request
- **AND** generation output is a real decode over the loaded weights, not a mock

#### Scenario: FlashInfer attention request is rejected before attention dispatch is wired
- **GIVEN** a model binding sets `hardware_strategy.flashinfer_attention: true`
- **AND** the runtime build does not yet have the decode-attention dispatch wired for that binding's architecture, device, or build
- **WHEN** the Candle LLM runtime loads the binding
- **THEN** loading fails with a typed `UnsupportedModel` error naming
  `candle-flashinfer-kernels::flashinfer_decode_attention`
- **AND** the runtime does not silently use the default attention path

#### Scenario: FlashInfer remains an optional dependency
- **WHEN** `core-host` is built without the `candle-flashinfer` feature
- **THEN** the FlashInfer-style Candle crate remains unlinked
- **AND** default and CPU-only AI inference builds are unchanged
