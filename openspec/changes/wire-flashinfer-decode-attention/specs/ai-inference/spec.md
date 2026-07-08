## MODIFIED Requirements

### Requirement: CUDA Graph and FlashInfer decode acceleration MUST be explicit and fail-closed
The AI inference build SHALL consume the pinned `astorise/candle` fork tag that
exposes `candle_core::CudaGraph` and the optional
`candle-flashinfer-kernels` crate for the downstream work proposed in
`huggingface/candle#3651`. Model deployments MAY declare
`hardware_strategy.cuda_graph_decode` and
`hardware_strategy.flashinfer_attention`. `cuda_graph_decode` SHALL continue
to be rejected until Tachyon's GPU decode loop has fixed-shape buffers and a
capture call site wired to `candle_core::CudaGraph`. `flashinfer_attention`
SHALL be enabled for a Llama-family checkpoint on a CUDA device once the
runtime's decode step is wired to `candle-flashinfer-kernels::flashinfer_decode_attention`;
every other architecture, non-CUDA device, or build predating that wiring
SHALL keep failing closed with the existing typed error, and prefill
(multi-token) forward passes SHALL always use the existing attention path
regardless of the flag, since `flashinfer_decode_attention` is a decode-only
kernel.

#### Scenario: CUDA Graph decode request is rejected before capture is wired
- **GIVEN** a model binding sets `hardware_strategy.cuda_graph_decode: true`
- **WHEN** the Candle LLM runtime loads the binding
- **THEN** loading fails with a typed `UnsupportedModel` error naming
  `candle_core::CudaGraph`
- **AND** the runtime does not silently execute the uncaptured decode loop

#### Scenario: FlashInfer attention request is rejected before attention dispatch is wired
- **GIVEN** a model binding sets `hardware_strategy.flashinfer_attention: true`
- **AND** the runtime build does not yet have the decode-attention dispatch wired for that binding's architecture, device, or build
- **WHEN** the Candle LLM runtime loads the binding
- **THEN** loading fails with a typed `UnsupportedModel` error naming
  `candle-flashinfer-kernels::flashinfer_decode_attention`
- **AND** the runtime does not silently use the default attention path

#### Scenario: FlashInfer attention is rejected on a non-Llama architecture or a non-CUDA device
- **GIVEN** a model binding sets `hardware_strategy.flashinfer_attention: true`
- **AND** the binding's architecture is not Llama, or the requested device is not CUDA
- **WHEN** the Candle LLM runtime loads the binding
- **THEN** loading fails with a typed `UnsupportedModel` error
- **AND** the runtime does not execute the default attention path as a fallback

#### Scenario: FlashInfer attention is enabled for a Llama binding on CUDA once decode-attention dispatch is available
- **GIVEN** the runtime build has the decode-step attention dispatch wired to `candle-flashinfer-kernels::flashinfer_decode_attention`
- **AND** a Llama model binding requesting a CUDA device sets `hardware_strategy.flashinfer_attention: true`
- **WHEN** the Candle LLM runtime loads the binding
- **THEN** the load succeeds and every decode step (one query token per sequence) runs through `flashinfer_decode_attention`
- **AND** prefill (multi-token) forward passes continue to use the existing attention path unchanged
- **AND** the model's weights and KV cache remain F32 (or whatever dtype was already requested) — unlike `paged_attention`, no dtype switch is required
- **AND** generation output is a real decode over the loaded weights, not a mock

#### Scenario: FlashInfer attention combined with paged attention is rejected rather than silently composed
- **GIVEN** a Llama model binding on CUDA sets both `hardware_strategy.flashinfer_attention: true` and `hardware_strategy.paged_attention: true`
- **WHEN** the Candle LLM runtime loads the binding
- **THEN** loading fails with a typed `UnsupportedModel` error naming the unsupported combination
- **AND** the runtime does not silently pick one of the two attention paths over the other

#### Scenario: FlashInfer remains an optional dependency
- **WHEN** `core-host` is built without the `candle-flashinfer` feature
- **THEN** the FlashInfer-style Candle crate remains unlinked
- **AND** default and CPU-only AI inference builds are unchanged
