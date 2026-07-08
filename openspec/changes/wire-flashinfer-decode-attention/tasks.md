## 1. External prerequisite (astorise/candle fork, separate repository)

- [ ] 1.1 Land an additive `use_flashinfer_attention` seam in `candle-transformers::models::llama` (proposed shape in `design.md` Decision 1): a `Config` flag mirroring `use_flash_attn`, taking effect only at the decode step (`seq_len == 1`), calling `flashinfer_decode_attention` on the pre-`repeat_kv` contiguous `Cache.kvs[block_idx]` K/V. Tracked upstream as [astorise/candle#11](https://github.com/astorise/candle/issues/11). Do not attempt to duplicate the Llama transformer stack in `core-host` as a workaround — same reasoning as `wire-paged-attention-decode-path`'s design.md Decision 2.
- [ ] 1.2 Tag a new fork release once that PR merges and its own tests pass.
- [ ] 1.3 Bump `core-host/Cargo.toml`'s pinned `astorise/candle` tag to the new release. Sections 2+ below are blocked until this lands.

## 2. Runtime wiring (blocked on Section 1)

- [ ] 2.1 In `try_load_with_topology` (`candle_llm_runtime.rs`), replace the unconditional `flashinfer_attention` rejection with an architecture/device gate mirroring `paged_attention`'s (`flashinfer_attention_supported = single_device_cuda_supported && requested_device != "cpu"`); keep the existing typed rejection for every other combination.
- [ ] 2.2 Reject the combination of `flashinfer_attention: true` and `paged_attention: true` with a typed error (see design.md Non-Goals) rather than silently picking one.
- [ ] 2.3 Thread the new `use_flashinfer_attention` config flag through `load_safetensors`'s Llama arm (set from `strategy.flashinfer_attention`) — no dtype change needed, unlike `paged_attention`'s BF16 switch.
- [ ] 2.4 Confirm LoRA adapters and speculative decoding either compose or are explicitly rejected for a flashinfer-attention deployment, following the same audit `wire-paged-attention-decode-path` Task 3.4 did (LoRA already reloads an independent CPU/F32 model per request, so it's likely unaffected; speculative decoding's verification path uses a fresh contiguous `Cache` from the shared model, same as before — confirm whether that still works given `use_flashinfer_attention` is a load-time config flag, not per-cache state like paged attention's `Option<PagedAttentionRuntime>`, so the dtype-mismatch risk that motivated paged attention's fallback may not apply here).

## 3. Tests

- [ ] 3.1 Update `flashinfer_attention_strategy_is_rejected_until_decode_attention_is_wired` (or equivalent) to assert the rejection still fires for non-Llama architectures and non-CUDA devices/builds after this change.
- [ ] 3.2 Add a test asserting the `flashinfer_attention` + `paged_attention` combination is rejected with a typed error.
- [ ] 3.3 Add a CUDA-gated (`candle-cuda` + `candle-flashinfer` features) test loading a real Llama checkpoint with `flashinfer_attention: true` and asserting `generate(...)` runs a real decode and returns non-empty, non-mocked output. Check whether the shared tiny fixture's dimensions are compatible first (unlike paged attention, no known block-size/head-dim constraint was found in the flashinfer kernel source — see design.md Decision/Risk notes — so the shared fixture may just work, but verify rather than assume).
- [ ] 3.4 Regression: `cargo test -p core-host --features ai-inference ai_inference::` (0 regressions), `cargo clippy --workspace --all-targets --features core-host/ai-inference -- -D warnings -D clippy::unwrap_used`, `RUSTFLAGS="-D dead_code" cargo check -p core-host --features ai-inference`, `cargo fmt --all -- --check`.

## 4. CI

- [ ] 4.1 Add a `cuda-quality` step exercising Task 3.3's flashinfer-attention generation on `arc-gpu-runners`, modeled on the paged-attention and single-device-CUDA proof steps.

## 5. Docs

- [ ] 5.1 Update `docs/ai-inference-candle-llm-runtime.md`'s "CUDA Graphs and FlashInfer Status" section to describe the now-enabled FlashInfer path (Llama+CUDA, decode-only, no dtype switch, rejected in combination with paged attention), leaving `cuda_graph_decode`'s status text as still-rejected.
- [ ] 5.2 `CHANGELOG.md` entry noting `hardware_strategy.flashinfer_attention` is now enabled for Llama/CUDA decode steps (still rejected elsewhere, and in combination with paged attention), and that this depended on an `astorise/candle` tag bump.
- [ ] 5.3 Update issue #312 status once this lands: flashinfer_attention done, `cuda_graph_decode` and continuous batching remain open.
