## Why

Discovered while starting `wire-paged-attention-decode-path` (issue #312):
every single-device (`GpuDistribution::Single`) loader in
`candle_llm_runtime.rs` — `load_safetensors`, `load_gguf`, the NVFP4 loader,
the LoRA-adapter loader — hardcodes `Device::Cpu` unconditionally, and
`try_load_with_topology` rejects any `single`-strategy request for a
non-`cpu` device before reaching any other check (covered today by the
regression test `single_strategy_still_rejects_a_gpu_device_request`, which
asserts this rejection fires even on a build with the `candle-cuda`
feature). Only the tensor/pipeline/expert-parallel engines
(`load_parallel`) construct real CUDA devices, and they use their own
duplicated cache implementation (`TensorParallelCache` et al.), not
`candle_transformers::models::llama::{Llama, Cache}` — the type the
`astorise/candle` fork's new paged-attention seam
([astorise/candle#8](https://github.com/astorise/candle/issues/8), tag
`tachyon-v0.11.0-3`) was added to.

This means `hardware_strategy.paged_attention` (and later
`cuda_graph_decode`, `flashinfer_attention` — both single-device, GPU-only
toggles) cannot be wired to anything reachable until the single-device Llama
path can actually run on a real CUDA device at all, dense/non-paged, as a
baseline. That baseline is this change's entire scope.

## What Changes

- `load_safetensors`'s Llama branch (and only the Llama branch — every
  other single-device architecture keeps loading on `Device::Cpu`
  unconditionally, unchanged) constructs `Device::cuda_if_available(0)`
  instead of `Device::Cpu` when: the binding requests a non-`cpu` device,
  the build has the `candle-cuda` feature compiled in, and the checkpoint's
  architecture is Llama. Every other combination (no `candle-cuda` feature,
  non-Llama architecture, or `cpu` requested) keeps today's behavior
  byte-for-byte, including the existing typed rejection.
- `VarBuilder::from_mmaped_safetensors` and `Llama::load` already take a
  `&Device` generically; no changes needed there beyond passing the real
  device through.
- Decode/prefill/tokenization/sampling call sites that assume CPU tensors
  implicitly (if any) are audited and fixed so a CUDA-resident `Llama` can
  actually `generate(...)`, not just load.
- Update `single_strategy_still_rejects_a_gpu_device_request` to scope its
  assertion to what still rejects (non-`candle-cuda` builds, non-Llama
  architectures) instead of asserting a blanket rejection.
- Add a CUDA-gated test proving a real Llama checkpoint loads and generates
  on a real CUDA device end to end (dense path, no paged attention).

## Capabilities

### New Capabilities
(none)

### Modified Capabilities
- `ai-inference`: the "GPU execution MUST be served when the candle CUDA
  backend is compiled in, and refused with a typed error otherwise"
  requirement changes from "the `single` path always returns the CPU-only
  error regardless of build" to "the `single` path executes on a real CUDA
  device for a Llama-family checkpoint when the `candle-cuda` feature is
  compiled in and a non-`cpu` device is requested; every other
  architecture/build combination keeps the existing rejection."

## Impact

- `core-host/src/ai_inference/candle_llm_runtime.rs`: `load_safetensors`'s
  device construction and the `try_load_with_topology` single-device
  rejection become architecture-aware instead of unconditional.
- `openspec/specs/ai-inference/spec.md`: requirement + scenario updates
  described above.
- Unblocks `wire-paged-attention-decode-path` (issue #312) Section 3+,
  and later `cuda_graph_decode`/`flashinfer_attention` follow-ups, all of
  which need this same single-device CUDA baseline.
- Out of scope: paged attention itself (tracked by
  `wire-paged-attention-decode-path`), GGUF/NVFP4/LoRA-adapter CUDA
  execution, and non-Llama architectures on the single-device path — all
  explicitly left on the existing CPU-only behavior for now.
