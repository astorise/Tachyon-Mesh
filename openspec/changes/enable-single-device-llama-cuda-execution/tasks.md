## 1. Device resolution

- [x] 1.1 In `load_safetensors`'s `ModelArchitecture::Llama` arm, resolve the device via `Device::cuda_if_available(0)` when `requested_device != "cpu"`. No explicit `#[cfg(feature = "candle-cuda")]` was needed here: `try_load_with_topology`'s gate (Task 1.2) already guarantees this branch is only reached when that feature is compiled in, so `Device::cuda_if_available` (which itself degrades to `Device::Cpu` when candle-core's own `cuda` feature isn't compiled) is safe to call unconditionally. Every other architecture arm is untouched.
- [x] 1.2 `try_load_with_topology`'s single-device rejection now reads `let single_device_cuda_supported = cfg!(feature = "candle-cuda") && architecture == ModelArchitecture::Llama;` and only rejects when that's `false` — still rejects when the build lacks `candle-cuda` or the architecture isn't Llama.
- [x] 1.3 Added `device: Device` to `SingleDeviceBackend::Llama { model, config, device }` (populated from 1.1, and at the two other construction sites — the NVFP4 fallback loader stays `Device::Cpu`) plus a `fn device(&self) -> Device` accessor returning `Device::Cpu` for every other variant.

## 2. Generate-time device threading

- [x] 2.1 `CandleLlmRuntime::decode()`'s `LoadedModel::Safetensors` arm now calls `backend.device()` instead of the outer hardcoded `Device::Cpu`; `Gguf` and `Parallel` arms untouched.
- [x] 2.2 Audited `llama_prefill_with_prefix_cache`, `run_prefill_chunks`, `decode_loop_from_logits`, `mask_row_for_fsm` — all already threaded the `device`/`input_device` parameter correctly (no stray `Device::Cpu`). Two more call sites needed the same `backend.device()` fix as 2.1: `last_logits_for_ids` (used by constrained/FSM-masked sampling) and the `#[cfg(test)]` `debug_last_logits` helper.

## 3. Tests

- [x] 3.1 Split `single_strategy_still_rejects_a_gpu_device_request`: gated `#[cfg(not(feature = "candle-cuda"))]`, plus a new always-on `single_strategy_still_rejects_a_gpu_device_request_for_a_non_llama_architecture` (Qwen2 fixture) proving the non-Llama case keeps rejecting on any build.
- [x] 3.2 Added `#[cfg(feature = "candle-cuda")]` test `single_device_llama_executes_on_a_real_cuda_device`. Not runnable on this dev machine (GPU present but local `nvcc`/MSVC toolchain mismatch, see design.md) — **verified on real hardware**: passed on `cuda-quality`/`arc-gpu-runners` in PR #341 (https://github.com/astorise/Tachyon-Mesh/actions/runs/28844467145/job/85545173627, step "Run single-device Llama CUDA execution proof: success").
- [x] 3.3 `cargo test -p core-host --features ai-inference ai_inference::` → 170 passed, 0 failed, 0 regressions. `cargo clippy --workspace --all-targets --features core-host/ai-inference -- -D warnings -D clippy::unwrap_used` → clean. `cargo fmt --all -- --check` → clean.

## 4. CI

- [x] 4.1 Added a `cuda-quality` step ("Run single-device Llama CUDA execution proof") running Task 3.2's test on `arc-gpu-runners`, right after the existing NCCL proof step. Ran green on PR #341 (all required checks — `quality`, `cuda-quality`, `security-audit`, `build-guests` — passing). An unrelated new advisory, RUSTSEC-2026-0204 (crossbeam-epoch), started failing `security-audit` on `main` itself mid-PR; ignored the same way prior advisories are (`ci.yml` + `deny.toml`), tracked with a removal reminder once a fixed crossbeam-epoch lands.

## 5. Docs

- [x] 5.1 Added a "Single-Device GPU Execution" section to `docs/ai-inference-candle-llm-runtime.md` and rewrote the stale comment at `candle_llm_runtime.rs` (the `try_load_with_topology` single-device gate) to describe the actual state.
- [x] 5.2 `CHANGELOG.md` entry added under `## Unreleased`.
