//! Tensor-parallel (Megatron-style) Llama forward pass.
//!
//! One substitution on top of `candle_transformers::models::llama::Llama`: the
//! MLP's up/gate (column-parallel) and down (row-parallel) projections are
//! sharded across `devices`, with a single all-reduce per transformer block —
//! the scope `design.md` §2 describes. Attention, embeddings and the LM head
//! are replicated (not sharded) and always run on `devices[0]` ("the primary
//! device"); only the MLP's compute and memory footprint are actually split
//! across the node's accelerators.
//!
//! This module used to reimplement the whole forward pass — cache, rotary
//! tables, causal mask, attention, block — because `Block`, `Mlp`,
//! `CausalSelfAttention` and `Cache` were private upstream. They are not any
//! more (candle #3809, #3817), so what remains here is the shard: an
//! implementation of `BlockMlp` and the loaders that build a `Llama` or a run
//! of `Block`s around it. The ~270 lines of copied attention math are gone, and
//! with them the risk that they drift from the reference they were copied from.

use candle_core::{Device, Result as CandleResult, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::llama::{BlockMlp, Config, Llama};

#[cfg(feature = "candle-cuda")]
use super::parallel::NcclShardGroup;
use super::parallel::{
    split_for_row_parallel, ColumnParallelLinear, NcclReducer, RowParallelLinear,
};

/// The KV cache and rotary tables every parallel variant here shares.
///
/// Upstream's own, now that it is reachable: a `Block` indexes it by
/// `block_idx`, which is exactly what a tensor-parallel model, a pipeline stage
/// holding a contiguous layer range, and a mixed dense/MoE stack all need.
pub(crate) type TensorParallelCache = candle_transformers::models::llama::Cache;

pub(crate) struct TensorParallelMlp {
    gate: ColumnParallelLinear,
    up: ColumnParallelLinear,
    down: RowParallelLinear,
    /// How `down`'s single per-layer all-reduce travels. Defaults to the
    /// host-staged sum; `with_nccl_group` swaps in the real collective.
    reducer: NcclReducer,
    devices: Vec<Device>,
    primary: Device,
}

impl TensorParallelMlp {
    /// `pub(crate)` so `expert_parallel_llama.rs` can build this exact dense
    /// MLP for a checkpoint's non-MoE layers, reusing its existing,
    /// numerically-verified math rather than reimplementing it.
    pub(crate) fn load(vb: VarBuilder, cfg: &Config, devices: &[Device]) -> CandleResult<Self> {
        let gate_w = vb
            .pp("gate_proj")
            .get((cfg.intermediate_size, cfg.hidden_size), "weight")?;
        let up_w = vb
            .pp("up_proj")
            .get((cfg.intermediate_size, cfg.hidden_size), "weight")?;
        let down_w = vb
            .pp("down_proj")
            .get((cfg.hidden_size, cfg.intermediate_size), "weight")?;
        Ok(Self {
            gate: ColumnParallelLinear::shard(&gate_w, devices)?,
            up: ColumnParallelLinear::shard(&up_w, devices)?,
            down: RowParallelLinear::shard(&down_w, devices)?,
            #[cfg(feature = "candle-cuda")]
            reducer: NcclReducer::new(None),
            #[cfg(not(feature = "candle-cuda"))]
            reducer: NcclReducer::new(),
            devices: devices.to_vec(),
            primary: devices[0].clone(),
        })
    }

    /// Attaches a shared NCCL communicator group (one per tensor-parallel
    /// shard group, built once by `TensorParallelLlama::load` and reused
    /// across every layer) to this MLP's `down` projection.
    #[cfg(feature = "candle-cuda")]
    fn with_nccl_group(mut self, group: Option<std::sync::Arc<NcclShardGroup>>) -> Self {
        self.reducer = NcclReducer::new(group);
        self
    }

    pub(crate) fn forward(&self, x: &Tensor) -> CandleResult<Tensor> {
        // `ColumnParallelLinear`/`RowParallelLinear` operate on 2-D
        // `[rows, features]` tensors; flatten the leading `(batch, seq)`
        // dims down to `rows` and restore the original shape afterwards.
        let in_shape = x.shape().clone();
        let hidden_size = in_shape.dims()[in_shape.dims().len() - 1];
        let x_2d = x.reshape((x.elem_count() / hidden_size, hidden_size))?;

        let gate_out = candle_nn::ops::silu(&self.gate.forward(&x_2d, &self.primary)?)?;
        let up_out = self.up.forward(&x_2d, &self.primary)?;
        let hidden = (gate_out * up_out)?;
        let x_shards = split_for_row_parallel(&hidden, &self.devices)?;
        let out_2d = self
            .down
            .forward_with_reducer(&x_shards, &self.primary, &self.reducer)?;

        let mut out_dims = in_shape.dims().to_vec();
        *out_dims.last_mut().expect("hidden_size dim exists") = out_2d.dim(1)?;
        out_2d.reshape(out_dims)
    }
}

/// The shard, as upstream's `Block` sees it.
///
/// A single-device `devices` slice degenerates the sharding to a plain dense
/// SwiGLU MLP, which is what a pipeline stage and an expert-parallel stack's
/// dense layers both rely on — one implementation covers all three.
impl BlockMlp for TensorParallelMlp {
    fn forward(&self, xs: &Tensor) -> CandleResult<Tensor> {
        TensorParallelMlp::forward(self, xs)
    }
}

/// Build one block's sharded MLP, ready to hand to
/// [`candle_transformers::models::llama::Block::load_with_block_mlp`].
///
/// `vb` must be rooted at the block, not at its MLP: the `mlp` prefix is
/// applied here so every caller — this module, pipeline stages, the dense
/// layers of an expert-parallel stack — agrees on it.
pub(crate) fn shard_block_mlp(
    vb: &VarBuilder,
    cfg: &Config,
    devices: &[Device],
    #[cfg(feature = "candle-cuda")] nccl_group: Option<std::sync::Arc<NcclShardGroup>>,
) -> CandleResult<Box<dyn BlockMlp>> {
    let mlp = TensorParallelMlp::load(vb.pp("mlp"), cfg, devices)?;
    #[cfg(feature = "candle-cuda")]
    let mlp = mlp.with_nccl_group(nccl_group);
    Ok(Box::new(mlp))
}

/// Tensor-parallel Llama: identical outputs to `Llama::load`/`forward` on the
/// same weights, with every block's MLP sharded across `devices`.
///
/// `devices` must have at least 2 entries; shape validation
/// (`parallel_topology::validate_plan_shape`) rejects tensor-parallel plans
/// with fewer before they reach this loader.
pub(crate) fn load_tensor_parallel_llama(
    vb: VarBuilder,
    cfg: &Config,
    devices: &[Device],
) -> CandleResult<Llama> {
    // One NCCL communicator group per tensor-parallel shard group, shared (via
    // `Arc`) across every layer's `RowParallelLinear` instead of paying
    // communicator-init cost per layer.
    #[cfg(feature = "candle-cuda")]
    let nccl_group = NcclShardGroup::try_new(devices).map(std::sync::Arc::new);
    Llama::load_with_mlp_factory(vb, cfg, |_layer_idx, mlp_vb| {
        let mlp = TensorParallelMlp::load(mlp_vb, cfg, devices)?;
        #[cfg(feature = "candle-cuda")]
        let mlp = mlp.with_nccl_group(nccl_group.clone());
        Ok(Box::new(mlp) as Box<dyn BlockMlp>)
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::ai_inference::candle_llm_runtime::write_tachyon_tiny_fixture;
    use candle_core::DType;
    use candle_transformers::models::llama::{
        Cache as DenseCache, Llama as DenseLlama, LlamaConfig,
    };

    fn load_config(root: &std::path::Path) -> Config {
        let raw = std::fs::read(root.join("config.json")).unwrap();
        let llama_config: LlamaConfig = serde_json::from_slice(&raw).unwrap();
        llama_config.into_config(false)
    }

    #[test]
    fn tensor_parallel_llama_matches_dense_reference_on_a_real_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        write_tachyon_tiny_fixture(dir.path()).unwrap();
        let config = load_config(dir.path());

        let weight_paths = vec![dir.path().join("model.safetensors")];

        let dense_device = Device::Cpu;
        let dense_vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&weight_paths, DType::F32, &dense_device)
        }
        .unwrap();
        let dense_model = DenseLlama::load(dense_vb, &config).unwrap();
        let mut dense_cache = DenseCache::new(false, DType::F32, &config, &dense_device).unwrap();

        let tp_devices = vec![Device::Cpu, Device::Cpu];
        let tp_vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&weight_paths, DType::F32, &tp_devices[0])
        }
        .unwrap();
        let tp_model = load_tensor_parallel_llama(tp_vb, &config, &tp_devices).unwrap();
        let mut tp_cache =
            TensorParallelCache::new(false, DType::F32, &config, &tp_devices[0]).unwrap();

        let input = Tensor::from_vec(vec![1u32, 2, 3], (1, 3), &dense_device).unwrap();

        let dense_logits = dense_model.forward(&input, 0, &mut dense_cache).unwrap();
        let tp_logits = tp_model.forward(&input, 0, &mut tp_cache).unwrap();

        let dense_logits: Vec<f32> = dense_logits.flatten_all().unwrap().to_vec1().unwrap();
        let tp_logits: Vec<f32> = tp_logits.flatten_all().unwrap().to_vec1().unwrap();
        assert_eq!(dense_logits.len(), tp_logits.len());
        for (a, b) in dense_logits.iter().zip(tp_logits.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} != {b}");
        }
    }

    /// The flag and the kernel must be enabled together.
    ///
    /// This used to assert something else: that setting `use_flash_attn` on a
    /// build without the kernel quietly fell back to naive attention. That
    /// leniency was a property of the attention this module reimplemented, and
    /// upstream deliberately does not have it — `flash_attn` is
    /// `unimplemented!()` when the feature is off, so a request for a kernel
    /// that was never compiled fails loudly instead of silently running
    /// something slower than asked for.
    ///
    /// Which is the better behaviour, and unreachable here: `candle-cuda`
    /// enables `candle-transformers/flash-attn` in the same feature list that
    /// flips this constant, so the two cannot disagree. That is the invariant
    /// worth pinning, and it is now checkable directly rather than through a
    /// forward pass.
    #[test]
    fn the_parallel_flash_attn_flag_tracks_the_compiled_kernel() {
        assert_eq!(
            crate::ai_inference::candle_llm_runtime::PARALLEL_LLAMA_USE_FLASH_ATTN,
            cfg!(feature = "candle-cuda"),
            "the flag must be set exactly when `candle-cuda` — and with it \
             `candle-transformers/flash-attn` — is compiled in"
        );
    }

    #[test]
    fn tensor_parallel_llama_with_a_single_device_matches_dense_reference() {
        // Regression guard for `multi_gpu: false` / `distribution_mode: single`
        // deployments (Task 7): a single-device plan must degenerate
        // `TensorParallelMlp`'s sharding to the dense case and produce output
        // identical to the pre-change dense engine, not just "close" output
        // from a real 2-device split.
        let dir = tempfile::tempdir().unwrap();
        write_tachyon_tiny_fixture(dir.path()).unwrap();
        let config = load_config(dir.path());
        let weight_paths = vec![dir.path().join("model.safetensors")];

        let dense_device = Device::Cpu;
        let dense_vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&weight_paths, DType::F32, &dense_device)
        }
        .unwrap();
        let dense_model = DenseLlama::load(dense_vb, &config).unwrap();
        let mut dense_cache = DenseCache::new(false, DType::F32, &config, &dense_device).unwrap();

        let tp_devices = vec![Device::Cpu];
        let tp_vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&weight_paths, DType::F32, &tp_devices[0])
        }
        .unwrap();
        let tp_model = load_tensor_parallel_llama(tp_vb, &config, &tp_devices).unwrap();
        let mut tp_cache =
            TensorParallelCache::new(false, DType::F32, &config, &tp_devices[0]).unwrap();

        let input = Tensor::from_vec(vec![1u32, 2, 3], (1, 3), &dense_device).unwrap();

        let dense_logits = dense_model.forward(&input, 0, &mut dense_cache).unwrap();
        let tp_logits = tp_model.forward(&input, 0, &mut tp_cache).unwrap();

        let dense_logits: Vec<f32> = dense_logits.flatten_all().unwrap().to_vec1().unwrap();
        let tp_logits: Vec<f32> = tp_logits.flatten_all().unwrap().to_vec1().unwrap();
        assert_eq!(dense_logits.len(), tp_logits.len());
        for (a, b) in dense_logits.iter().zip(tp_logits.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} != {b}");
        }
    }

    #[test]
    fn tensor_parallel_llama_decodes_a_second_token_with_kv_cache() {
        let dir = tempfile::tempdir().unwrap();
        write_tachyon_tiny_fixture(dir.path()).unwrap();
        let config = load_config(dir.path());
        let weight_paths = vec![dir.path().join("model.safetensors")];

        let dense_device = Device::Cpu;
        let dense_vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&weight_paths, DType::F32, &dense_device)
        }
        .unwrap();
        let dense_model = DenseLlama::load(dense_vb, &config).unwrap();
        let mut dense_cache = DenseCache::new(true, DType::F32, &config, &dense_device).unwrap();

        let tp_devices = vec![Device::Cpu, Device::Cpu];
        let tp_vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&weight_paths, DType::F32, &tp_devices[0])
        }
        .unwrap();
        let tp_model = load_tensor_parallel_llama(tp_vb, &config, &tp_devices).unwrap();
        let mut tp_cache =
            TensorParallelCache::new(true, DType::F32, &config, &tp_devices[0]).unwrap();

        let prompt = Tensor::from_vec(vec![1u32, 2, 3], (1, 3), &dense_device).unwrap();
        dense_model.forward(&prompt, 0, &mut dense_cache).unwrap();
        tp_model.forward(&prompt, 0, &mut tp_cache).unwrap();

        let next = Tensor::from_vec(vec![0u32], (1, 1), &dense_device).unwrap();
        let dense_logits = dense_model.forward(&next, 3, &mut dense_cache).unwrap();
        let tp_logits = tp_model.forward(&next, 3, &mut tp_cache).unwrap();

        let dense_logits: Vec<f32> = dense_logits.flatten_all().unwrap().to_vec1().unwrap();
        let tp_logits: Vec<f32> = tp_logits.flatten_all().unwrap().to_vec1().unwrap();
        for (a, b) in dense_logits.iter().zip(tp_logits.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} != {b}");
        }
    }
}
