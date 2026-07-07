## Context

`candle_llm_runtime.rs`'s single-device loaders all hardcode `Device::Cpu`:
`load_safetensors:1108`, `load_gguf:1443`, the NVFP4 loader:~1348, the
LoRA-adapter loader:~1843. `try_load_with_topology` additionally rejects
any `distribution_mode: single` request for a non-`cpu` device
unconditionally (line 1037), and the per-request `decode()` dispatcher
rebuilds `let device = Device::Cpu;` again at line 2196 for the Safetensors
and GGUF match arms (the parallel arms already override this with their own
`primary` device from `devices[0]`). A named regression test,
`single_strategy_still_rejects_a_gpu_device_request`, currently asserts this
rejection fires even on a `candle-cuda` build — this is documented,
intentional behavior today, not an oversight, so changing it means updating
that test's contract deliberately, not routing around it.

Only the tensor/pipeline/expert-parallel engines (`load_parallel`) build
real `Device::Cuda` handles today, via `Device::cuda_if_available`
(`parallel.rs:78`), and they use their own duplicated cache type
(`TensorParallelCache`), not `candle_transformers::models::llama::Cache`.

## Goals / Non-Goals

**Goals:**
- Give the single-device Llama safetensors path a real CUDA device when:
  the build has the `candle-cuda` feature, the binding requests a non-`cpu`
  device, and the checkpoint's architecture is Llama.
- Keep every other combination — no `candle-cuda` feature, non-Llama
  architecture on the single-device path, or `cpu` requested — on today's
  behavior byte-for-byte, including the existing typed rejection.
- Make the resolved device available at generate-time (not just load-time),
  so prefill/decode tensor construction matches the device the weights
  actually live on.

**Non-Goals:**
- Paged attention, CUDA graph decode, FlashInfer decode — those are
  `wire-paged-attention-decode-path` (issue #312) and its follow-ups, built
  on top of this baseline.
- GGUF, NVFP4, and LoRA-adapter execution on CUDA — stay CPU-only for now;
  same treatment can follow later using the same pattern.
- Non-Llama architectures on the single-device path (Qwen2/3, Gemma2/3,
  Phi3/4, DeepSeek family) — stay CPU-only for now.
- Multi-GPU/parallel execution — already has a real CUDA path, untouched.

## Decisions

### 1. Device resolution stays local to the Llama branch, gated by feature + architecture
In `load_safetensors`, the `ModelArchitecture::Llama` arm resolves its
device as:

```rust
#[cfg(feature = "candle-cuda")]
let device = if architecture == ModelArchitecture::Llama && requested_device != "cpu" {
    Device::cuda_if_available(0).map_err(invalid_weights)?
} else {
    Device::Cpu
};
#[cfg(not(feature = "candle-cuda"))]
let device = Device::Cpu;
```

placed inside (or threaded into) the Llama match arm specifically, so every
other architecture's branch keeps its own unconditional `Device::Cpu`
untouched. `Device::cuda_if_available` silently falls back to `Device::Cpu`
if the feature is compiled in but no physical device is found at runtime —
this matches the existing parallel-path convention
(`discover_cluster_topology`, `parallel.rs:78`) rather than inventing a new,
inconsistent fail-closed rule just for this path.

Alternative considered: hard-error if `candle-cuda` is compiled but no
physical CUDA device is present. Rejected for consistency — the parallel
engines already silently degrade to CPU in that case, and diverging here
would make otherwise-identical hardware situations behave differently
depending on `distribution_mode`.

### 2. `try_load_with_topology`'s single-device rejection becomes architecture-aware
Replace the unconditional `strategy.distribution_mode == GpuDistribution::Single && requested_device != "cpu"` rejection with a check that only fires when the binding will NOT get a real device from Decision 1 — i.e., keep rejecting when the build lacks `candle-cuda`, or the architecture isn't Llama (architecture is known slightly later in the current control flow, once `resolve_model_format`/`inspect_model_architecture` have run, so this rejection point may need to move a few lines later than its current position, after the architecture is known but before weights are loaded — matching how the existing `paged_attention`/`cuda_graph_decode`/`flashinfer_attention` checks already sit after architecture/format resolution).

### 3. Track the resolved device on `SingleDeviceBackend::Llama`, not as a second Runtime-wide field
Add a `device: Device` field to the `Llama { model, config }` variant of
`SingleDeviceBackend`, and a `fn device(&self) -> Device` accessor
(`Device::Cpu` for every other variant). `decode()`'s Safetensors match arm
calls `backend.device()` instead of relying on the outer `let device =
Device::Cpu;`. This keeps the device tied to the specific loaded backend
instance rather than adding ambient state to `CandleLlmRuntime` that every
other (CPU-only) architecture would have to ignore.

Alternative considered: add `device: Device` directly to `CandleLlmRuntime`.
Rejected — `CandleLlmRuntime` is shared across all `LoadedModel` variants
(Safetensors/Gguf/Parallel), and the parallel arms already source their
device from `devices[0]` per-request; a second, mostly-unused top-level
field would be redundant and easy to drift out of sync.

### 4. Scope is Llama only, matching how other recent runtime work incrementally covers Llama first
Consistent with the NVFP4 fallback path and `wire-paged-attention-decode-path`'s
own scoping: broadening to other architectures is a mechanical repeat of
this same pattern per architecture, left for later changes so this one
stays reviewable.

## Risks / Trade-offs

- **[Risk] `single_strategy_still_rejects_a_gpu_device_request` is a named
  contract test asserting the opposite of this change's goal.** →
  Mitigation: split it explicitly — keep a `#[cfg(not(feature =
  "candle-cuda"))]` variant (or an architecture-varied fixture) proving the
  rejection still fires for the cases that must keep rejecting, and add a
  new `#[cfg(feature = "candle-cuda")]` test for the now-succeeding
  Llama+CUDA case, rather than silently deleting or weakening the original
  assertion.
- **[Risk] Downstream tensor construction (rotary cos/sin, causal mask,
  input-ids tensor) must be built on the same device as the loaded
  weights, or the first `forward()` call errors on a device mismatch.** →
  Mitigation: `Cache::new(..., device)` and prefill/decode already take
  `device` as a parameter (`candle_llm_runtime.rs:387`); Decision 3 makes
  sure the *correct* device reaches that parameter instead of the
  hardcoded `Device::Cpu` at line 2196.
- **[Risk] No local CUDA compile validation possible right now** (this dev
  machine's CUDA 13.2 toolkit doesn't pair with its installed MSVC/VS "18"
  toolset — `nvcc` fails every kernel with "Host compiler targets
  unsupported OS"; see `wire-paged-attention-decode-path`'s design.md for
  the same finding). → Mitigation: `cuda-quality` (`arc-gpu-runners`,
  Linux) is the validation gate, same as every other CUDA-gated change in
  this repo.
- **[Trade-off] Only ordinal `0` is used for the single-device CUDA case**
  (no multi-GPU selection at this layer — that is what
  `distribution_mode: tensor_parallelism`/`device_ids` is for). Accepted:
  matches `ModelDevice`'s existing shape (no ordinal field), and
  multi-device placement already has its own dedicated path.
