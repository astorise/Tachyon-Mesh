# Design: Layer-Wise Inference & Pipeline

## Approach

All four implementation workstreams land in `core-host/src/ai_inference.rs` plus the WIT contract in `wit/ai/inference.wit`. No changes to the Axum request path or the existing `AiInferenceRuntime` boot logic are required — the new types are self-contained and integrated at model-load / call-site level.

### 1. WIT contract (`wit/ai/inference.wit`)

The existing `inference-request` record is unchanged. Two new types are added to the interface:
- `enum memory-profile { performance, layer-wise-streaming }` — opt-in at the call site.
- `record inference-options { temperature: f32, max-tokens: u32, profile: memory-profile }` — groups per-call tuning knobs.
- `infer-with-options` — new exported function that accepts explicit options; `infer` remains as a stable backwards-compatible entry point.

### 2. Zero-copy model loader (`LayerWiseMappedModel`)

`memmap2 = "0.9"` is added to `core-host/Cargo.toml`. `memmap2::Mmap` maps the `.safetensors` file into the process address space with `mmap(2)` / `CreateFileMapping` under the hood. The OS page cache services reads; no heap allocation occurs for the weight bytes.

`LayerWiseMappedModel::load_layer(idx)` slices the mapped region into equal-sized chunks (one per transformer layer), reinterprets the bytes as little-endian `f32`, and builds a `CandleTensor` on `Device::Cpu`. The tensor data is a copy of the page-cache pages — acceptable because the copy happens only once per layer per forward pass, and the original mmap bytes are promptly eligible for eviction after the copy.

`unsafe impl Send/Sync` is sound because the mmap is opened read-only (`Mmap`, not `MmapMut`) and the lifetime of the raw pointer is tied to the struct itself.

### 3. Prefill batching — Phase 1 (`PrefillBatch`)

`PrefillBatch::run` iterates over all `num_layers` layers in sequence:
1. Calls `loader.load_layer(i)` — O(layer_size) copy from page cache.
2. Performs the mock forward pass (addition-based stub; real impl: Candle transformer block call).
3. Extracts the KV-Cache from the hidden states and stores it in a `KvCacheSlice` (Host RAM).
4. Drops `LayerWeightSlice` — simulated VRAM free.

The `KvCacheSlice` is a `Vec<Vec<f32>>` resident on the heap (host RAM). In a production build with a real CUDA backend, step 2 would synchronise a CUDA stream before step 3.

### 4. Async pipeline ring buffer — Phase 2 (`LayerPipeline`)

`LayerRingBuffer` is a fixed-capacity circular array of `Option<CandleTensor>`. `window` slots are maintained; when slot N+1 is pushed, slot N-window is evicted. This mirrors the pre-allocated double-buffer pattern used in CUDA prefetch pipelines.

`LayerPipeline::decode` runs the autoregressive token-generation loop:
- **Compute stream** (synchronous in CPU build): forward pass through layer N using the cached ring-buffer tensor.
- **Copy stream** (pre-fetch): `load_layer(N+window)` populates the next ring slot before the compute step needs it. On a real CUDA build this would be a `cudaMemcpyAsync` on a separate `cudaStream_t`.
- **KV-Cache page-out**: after computing layer N, the updated KV entry is appended to `KvCacheSlice[N]` in Host RAM via `append_token`.
- **VRAM eviction**: `evict_oldest()` frees the ring slot two positions behind the window.

This maintains an O(1) VRAM footprint per layer: at most `window` layer tensors reside in VRAM simultaneously, and KV-Cache grows only in host RAM.

## Trade-offs

| Decision | Chosen | Rejected | Reason |
|---|---|---|---|
| mmap slice reinterpretation | `f32::from_le_bytes` per-chunk | `bytemuck` cast | Zero extra dependencies; acceptable for layer-sized allocations |
| KV-Cache representation | `Vec<Vec<f32>>` heap | GPU-pinned buffers | CPU-resident store is the spec requirement; pinned memory is an optimisation |
| Ring buffer eviction | index-based `head - capacity` | LRU tracking | Access pattern is strictly sequential; LRU tracking overhead unnecessary |
| Async streams | Sequential CPU fallback | `cudarc` or Tokio async | Current backend is CPU mock; the structural contract is correct for future GPU wiring |
| PrefillBatch forward pass | Addition stub | Full Candle attention block | Keeps the PR compilable without a full LLM architecture; the layer-iteration contract is the deliverable |
