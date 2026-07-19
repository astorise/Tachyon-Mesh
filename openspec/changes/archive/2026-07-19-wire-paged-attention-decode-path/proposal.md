## Why

Issue #312 (audit evolution 3.4): `hardware_strategy.paged_attention`,
`cuda_graph_decode`, and `flashinfer_attention` are declarable end-to-end
(manifest, schema, UI, MCP) but the Candle LLM runtime rejects every one of
them at load time — `try_load_with_topology` returns a typed
`UnsupportedModel` naming the missing wiring
(`core-host/src/ai_inference/candle_llm_runtime.rs:1003-1029`), matching the
existing `ai-inference` spec requirements that these MUST fail closed until
wired. Paged attention is the prerequisite for the other three (CUDA graph
decode wants fixed-shape steady-state buffers, which paged block storage
gives for free; continuous batching needs block-granularity admission), so
it is the highest-leverage first step and the scope of this change.
`cuda_graph_decode`, `flashinfer_attention`, and continuous batching stay
rejected and are tracked as follow-up changes once this lands.

## What Changes

- Add a Tachyon-owned CUDA block allocator and per-sequence block table
  (`core-host`, new module) sized from the deployment's KV-cache budget:
  fixed-size KV blocks, a free-list allocator, and a table mapping each
  sequence's logical block index to a physical block id, matching the layout
  `candle_flash_attn::flash_attn_varlen_paged_windowed` expects
  (`seqlens_q`, `seqlens_k`, `block_table`, `page_block_size`).
- **Prerequisite in the forked `astorise/candle` repo (not this repo) — done:**
  `candle-transformers::models::llama::{Cache, CausalSelfAttention, Block}`
  were private/non-`pub` and hard-coded the contiguous concat-and-narrow KV
  path (`candle-transformers/src/models/llama.rs:396-450`), so no downstream
  crate could hook in paged storage without duplicating the whole transformer
  stack. Filed as [astorise/candle#8](https://github.com/astorise/candle/issues/8);
  landed additively on tag `tachyon-v0.11.0-3` as `Cache::set_paged_kv`/
  `paged_kv` per-layer slots (see `design.md` Decision 2) rather than the
  originally-proposed `Cache::Paged(..)` enum — same additive, zero-regression
  effect, smaller diff. Tachyon-Mesh work below can now proceed.
- Once the fork tag is available: wire `CandleLlmRuntime` to build the block
  allocator/table at load time when `hardware_strategy.paged_attention` is
  set on a Llama-family CUDA deployment, thread the block table into
  decode/prefill instead of the contiguous cache, and remove the
  unconditional rejection for that architecture (other architectures keep
  rejecting until they get the same treatment).
- Update the `ai-inference` spec's PagedAttention requirement from
  "always rejected" to "rejected unless the block allocator/table path is
  available for the requested architecture and device", with scenarios for
  both the still-rejected (non-Llama, or CPU device) and now-enabled
  (Llama on CUDA) cases.
- Add a GPU CI proof (`cuda-quality` job, `arc-gpu-runners`) exercising a
  real paged-attention generation, modeled on the existing NCCL/NVML proof
  job, plus a before/after decode-throughput bench entry (issue #308).

## Capabilities

### New Capabilities
(none — paged attention is already a documented capability of `ai-inference`)

### Modified Capabilities
- `ai-inference`: the "PagedAttention MUST require an explicit block-table
  runtime path" requirement changes from an unconditional fail-closed
  rejection to a real block-paged execution path for Llama-family CUDA
  deployments, with fail-closed rejection preserved for every other
  architecture/device combination.

## Impact

- `core-host/src/ai_inference/candle_llm_runtime.rs`: new block
  allocator/table module, `try_load_with_topology` gains a
  Llama+CUDA+`paged_attention` branch instead of an unconditional error,
  decode/prefill call sites for Llama switch to the paged path when enabled.
- `core-host/Cargo.toml`: bumps the pinned `astorise/candle` git tag to
  `tachyon-v0.11.0-3` and enables `candle-transformers/flash-attn` under the
  `candle-cuda` feature.
- `openspec/specs/ai-inference/spec.md`: requirement + scenario updates
  described above.
- `.github/workflows/ci.yml`: new `cuda-quality` step for the paged-attention
  proof.
- **External dependency — resolved**: `astorise/candle`'s additive paged-cache
  seam landed on tag `tachyon-v0.11.0-3` ([astorise/candle#8](https://github.com/astorise/candle/issues/8)).
- Out of scope here (tracked as follow-ups once this lands):
  `cuda_graph_decode`, `flashinfer_attention`, continuous batching.
