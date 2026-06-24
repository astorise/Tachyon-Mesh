# Design: MoE Checkpoint Loader for Expert-Parallel Dispatch

## 1. Current state (what exists today)
```rust
// candle_llm_runtime.rs:596 — runs unconditionally, before any strategy dispatch
let config = Self::load_llama_config(alias, root)?;
...
match strategy.distribution_mode {
    GpuDistribution::TensorParallelism => { /* real, wired */ }
    GpuDistribution::PipelineParallelism => { /* real, wired (prefill-only) */ }
    GpuDistribution::ExpertParallelism => Err(CandleLlmError::UnsupportedModel { detail: "... not yet implemented ...", .. }),
    GpuDistribution::Single => unreachable!(...),
}
```
`load_llama_config` (line 662) parses `config.json` via `ModelTypeProbe { model_type: String }` and hard-rejects anything where `model_type != "llama"` (line 678) — this runs *before* the `match`, so a real Mixtral checkpoint (`model_type: "mixtral"`) never reaches the `ExpertParallelism` arm at all; it fails earlier with a generic "expected a Llama-family checkpoint" error. This is the first thing this change must address, not just the explicit rejection arm.

`ExpertParallelMlp::load` (`parallel.rs:968`) and its supporting types are fully built and tested but have zero callers:
```rust
pub(crate) struct ExpertPlacementPlan { expert_device_index: BTreeMap<u32, usize> }
impl ExpertPlacementPlan {
    pub(crate) fn round_robin(expert_count: u32, device_count: usize) -> Self { .. }
    pub(crate) fn device_index_for(&self, expert_id: u32) -> Option<usize> { .. }
}
pub(crate) fn detect_expert_count<'a>(tensor_names: impl Iterator<Item = &'a str>, layer_idx: usize) -> Option<usize>;
pub(crate) struct ExpertMlp { w1: Linear, w3: Linear, w2: Linear, device: Device }
pub(crate) struct ExpertParallelMlp { gate: Tensor, experts: Vec<ExpertMlp> }
impl ExpertParallelMlp {
    pub(crate) fn load(vb, hidden_size, intermediate_size, num_experts, plan: &ExpertPlacementPlan, devices: &[Device]) -> CandleResult<Self>;
    pub(crate) fn forward(&self, x: &Tensor) -> CandleResult<Tensor>; // x: [tokens, hidden] 2-D
}
```

## 2. Target state

### 2.1 MoE config parsing
Add a `MoeConfig` struct capturing the HF Mixtral `config.json` fields this loader needs:
```rust
#[derive(serde::Deserialize)]
struct MixtralConfigJson {
    model_type: String,           // must be "mixtral"
    hidden_size: usize,
    intermediate_size: usize,
    num_hidden_layers: usize,
    num_attention_heads: usize,
    num_key_value_heads: usize,
    vocab_size: usize,
    rms_norm_eps: f64,
    num_local_experts: usize,     // NEW: expert count per layer
    num_experts_per_tok: usize,   // NEW: must be 1 for this change (Non-Goal: top-k>1)
    // rope_theta, max_position_embeddings, etc. mirrored from the existing LlamaConfig parse
}
```
`ModelTypeProbe`'s existing `model_type` field is reused to branch *before* calling `load_llama_config`: if `probe.model_type == "mixtral"`, parse `MixtralConfigJson` and take the MoE load path; if `"llama"`, the existing path is completely unchanged. This keeps the dense/tensor/pipeline paths byte-for-byte unaffected — the branch is additive, not a modification of the existing check.

`num_experts_per_tok != 1` is rejected with a typed `UnsupportedModel` error at config-parse time (fail fast, per the Non-Goals top-1-only scope), rather than silently truncating to top-1.

### 2.2 Per-layer dense-vs-MoE detection
```rust
let tensor_names: Vec<String> = /* from the safetensors header, already available via existing weight-loading machinery */;
for layer_idx in 0..moe_config.num_hidden_layers {
    match detect_expert_count(tensor_names.iter().map(String::as_str), layer_idx) {
        Some(expert_count) => {
            // load ExpertParallelMlp for this layer's MLP
        }
        None => {
            // load the existing dense TensorParallelMlp for this layer's MLP
        }
    }
}
```
This makes mixed dense/MoE checkpoints (some real architectures keep early/late layers dense) load correctly without any special-casing beyond what `detect_expert_count` already provides per layer.

### 2.3 `ExpertPlacementPlan` from `hardware_strategy.expert_device_map`
`hardware_strategy.expert_device_map: list<tuple<u32, u32>>` (expert id → device-ids index) already exists in the WIT-defined `HardwareStrategy` and is threaded into `IntegrityModelBinding` per `2026-06-22-wire-model-parallel-runtime-dispatch`. Add a constructor:
```rust
impl ExpertPlacementPlan {
    /// Builds a plan from an explicit expert->device-index map (deployment
    /// override); falls back to `round_robin` for any expert id the map
    /// omits, so a deployment can pin a subset of experts without having to
    /// enumerate all of them.
    pub(crate) fn from_explicit_map_or_round_robin(
        expert_device_map: &[(u32, u32)],
        expert_count: u32,
        device_count: usize,
    ) -> Self {
        let mut plan = Self::round_robin(expert_count, device_count);
        for &(expert_id, device_index) in expert_device_map {
            plan.expert_device_index.insert(expert_id, device_index as usize);
        }
        plan
    }
}
```

### 2.4 `ExpertParallelLlama` model wrapper
A new struct, structurally parallel to `TensorParallelLlama`/`PipelineParallelLlama`:
```rust
pub(crate) struct ExpertParallelLlama {
    wte: Embedding,
    layers: Vec<MoeOrDenseBlock>, // enum { Dense(TensorParallelBlock), Moe { attn: ReplicatedAttention, mlp: ExpertParallelMlp, .. } }
    ln_f: RmsNorm,
    lm_head: Linear,
    cache: TensorParallelCache, // shared KV cache machinery, reused unchanged
}
```
`MoeOrDenseBlock`'s dense variant reuses `TensorParallelBlock` entirely unchanged (attention + norms + dense MLP); the MoE variant reuses `TensorParallelBlock`'s attention/norm sub-components directly (they are unaffected by MoE) but swaps in `ExpertParallelMlp::forward` for the MLP step. `forward(&mut self, index_pos, input)` mirrors `TensorParallelLlama::forward`'s existing prefill/decode signature so `candle_llm_runtime.rs`'s dispatch can drive it identically to the tensor-parallel path (and, once `2026-06-23-pipeline-parallel-decode-kv-cache` lands, identically to the pipeline-parallel path).

### 2.5 `candle_llm_runtime.rs` dispatch
```rust
GpuDistribution::ExpertParallelism => {
    let model = ExpertParallelLlama::load(&weight_paths, DType::F32, &moe_config, &plan.expert_device_map, &devices)
        .map_err(invalid)?;
    Ok((LoadedModel::Parallel(ParallelModel::Expert { model: Box::new(model), config: moe_config, eos_tokens, devices }), limits))
}
```
replacing the current unconditional `Err(...)`. `LoadedModel`/`ParallelModel`'s generation dispatch gains an `Expert` arm structured like the existing `Tensor` arm (prefill + per-token decode loop, reusing the same sampling/stop-condition code).

## 3. Why config parsing must branch before, not after, `load_llama_config`
`load_llama_config` is shared by the dense single-GPU path and the tensor/pipeline-parallel paths (it's called unconditionally at `try_load_with_topology` line 596, before the `match`). Modifying it to also accept `model_type: "mixtral"` and return a Llama `Config` would be incorrect — Mixtral's config shape genuinely differs (it has `num_local_experts`/`num_experts_per_tok`, no single dense `intermediate_size`-only MLP), and candle_transformers' own `Config`/`LlamaConfig` types have no field for expert count. The cleanest, lowest-risk approach is an early branch on `probe.model_type` before calling `load_llama_config` at all, so the existing function and its callers (dense + tensor + pipeline paths) are provably untouched.

## 4. Out of scope for this change
- Top-k > 1 routing (Non-Goal, §0 above).
- Combining expert-parallelism with tensor- or pipeline-parallelism in a single deployment.
- Any change to `ExpertPlacementPlan::round_robin`, `detect_expert_count`, `ExpertMlp`, or `ExpertParallelMlp::forward`'s existing, already-tested logic.
