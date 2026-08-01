//! Expert-parallel (Mixtral-style MoE) Llama forward pass.
//!
//! A checkpoint loaded under `distribution_mode: expert_parallelism` is a
//! per-layer mix of dense transformer blocks and MoE blocks: each layer is
//! classified independently by [`super::parallel::detect_expert_count`]
//! scanning the checkpoint's own tensor names for the
//! `<prefix>.layers.<i>.*.experts.<id>.*` naming convention, with no
//! requirement that every layer be the same kind (mirrors real Mixtral-family
//! checkpoints, where every layer happens to be MoE, while still tolerating
//! mixed dense/MoE checkpoints without a separate code path). Attention is
//! always dense and replicated — [`super::tensor_parallel_llama::ReplicatedAttention`]
//! is reused unmodified rather than reimplemented, since MoE only changes the
//! MLP. A dense layer's MLP reuses
//! [`super::tensor_parallel_llama::TensorParallelMlp`] unchanged (the same
//! sharded-or-dense dense-MLP path `TensorParallelBlock` already uses); an
//! MoE layer's MLP is [`super::parallel::ExpertParallelMlp`], which already
//! implements real gate-then-route dispatch across the experts placed by an
//! [`super::parallel::ExpertPlacementPlan`].
//!
//! Like `TensorParallelLlama` and `PipelineParallelLlama::forward_with_cache`,
//! the KV cache is owned by the caller (one [`TensorParallelCache`] per
//! generation request, shared across every layer regardless of whether that
//! layer is dense or MoE — the cache only cares about `block_idx`, not which
//! kind of MLP follows attention), not stored as a field on this struct, so
//! concurrent requests against the same loaded model never share or corrupt
//! each other's KV state.

use candle_core::{DType, Device, IndexOp, Result as CandleResult, Tensor};
use candle_nn::{Module, VarBuilder};
use candle_transformers::models::llama::Config;
use candle_transformers::models::with_tracing::{
    linear_no_bias as linear, Embedding, Linear, RmsNorm,
};

use super::parallel::{detect_expert_count, ExpertParallelMlp, ExpertPlacementPlan};
use super::tensor_parallel_llama::{TensorParallelCache, TensorParallelMlp};
use candle_transformers::models::llama::{Block, BlockMlp};

/// A transformer block's MLP: either dense or MoE, selected per-layer at load
/// time from the checkpoint's own tensor names. The dense variant reuses
/// `TensorParallelMlp` unchanged (it already degenerates to a plain dense
/// SwiGLU MLP for a single-device `devices` slice, exactly as
/// `TensorParallelBlock`'s own dense path does).
enum ExpertOrDenseBlock {
    Dense(TensorParallelMlp),
    Moe(ExpertParallelMlp),
}

impl BlockMlp for ExpertOrDenseBlock {
    fn forward(&self, x: &Tensor) -> CandleResult<Tensor> {
        match self {
            Self::Dense(mlp) => mlp.forward(x),
            Self::Moe(mlp) => {
                // `ExpertParallelMlp::forward` operates on 2-D `[tokens,
                // hidden]` activations; flatten the leading `(batch, seq)`
                // dims and restore them afterwards, exactly as
                // `TensorParallelMlp::forward` does for its own sharded
                // projections.
                let in_shape = x.shape().clone();
                let hidden_size = in_shape.dims()[in_shape.dims().len() - 1];
                let x_2d = x.reshape((x.elem_count() / hidden_size, hidden_size))?;
                let out_2d = mlp.forward(&x_2d)?;
                out_2d.reshape(in_shape)
            }
        }
    }
}

/// Expert-parallel Llama/Mixtral-style model: identical attention/embedding/
/// LM-head math to [`super::tensor_parallel_llama::TensorParallelLlama`], with
/// each layer's MLP independently dense or MoE per [`detect_expert_count`].
pub(crate) struct ExpertParallelLlama {
    wte: Embedding,
    blocks: Vec<Block>,
    ln_f: RmsNorm,
    lm_head: Linear,
}

impl ExpertParallelLlama {
    /// `tensor_names` is every tensor name in the checkpoint (e.g. from
    /// `MmapedSafetensors::multi(...).tensors()`), used to classify each
    /// layer as dense or MoE before that layer's weights are loaded.
    pub(crate) fn load<'a>(
        vb: VarBuilder,
        cfg: &Config,
        tensor_names: impl Iterator<Item = &'a str> + Clone,
        plan: &ExpertPlacementPlan,
        devices: &[Device],
    ) -> CandleResult<Self> {
        let wte = Embedding::new(cfg.vocab_size, cfg.hidden_size, vb.pp("model.embed_tokens"))?;
        let lm_head = if cfg.tie_word_embeddings {
            Linear::from_weights(wte.embeddings().clone(), None)
        } else {
            linear(cfg.hidden_size, cfg.vocab_size, vb.pp("lm_head"))?
        };
        let ln_f = RmsNorm::new(cfg.hidden_size, cfg.rms_norm_eps, vb.pp("model.norm"))?;
        // Upstream blocks throughout: attention, the norms and the residual
        // structure are candle's, and only the feed-forward differs — which is
        // the whole of what "expert parallel" means here. The per-layer choice
        // between dense and MoE is made by the factory, layer by layer, which
        // is exactly the granularity `detect_expert_count` reports at.
        let blocks = (0..cfg.num_hidden_layers)
            .map(|i| {
                let expert_count = detect_expert_count(tensor_names.clone(), i);
                let block_vb = vb.pp(format!("model.layers.{i}"));
                let mlp: Box<dyn BlockMlp> = match expert_count {
                    Some(num_experts) => {
                        Box::new(ExpertOrDenseBlock::Moe(ExpertParallelMlp::load(
                            block_vb.pp("block_sparse_moe"),
                            cfg.hidden_size,
                            cfg.intermediate_size,
                            num_experts,
                            plan,
                            devices,
                        )?))
                    }
                    None => Box::new(ExpertOrDenseBlock::Dense(TensorParallelMlp::load(
                        block_vb.pp("mlp"),
                        cfg,
                        devices,
                    )?)),
                };
                Block::load_with_block_mlp(block_vb, cfg, i, mlp)
            })
            .collect::<CandleResult<Vec<_>>>()?;
        Ok(Self {
            wte,
            blocks,
            ln_f,
            lm_head,
        })
    }

    pub(crate) fn forward(
        &self,
        x: &Tensor,
        index_pos: usize,
        cache: &mut TensorParallelCache,
    ) -> CandleResult<Tensor> {
        let (_b_sz, seq_len) = x.dims2()?;
        let mut x = self.wte.forward(x)?;
        for (block_idx, block) in self.blocks.iter().enumerate() {
            x = block.forward(&x, index_pos, block_idx, cache, None)?;
        }
        let x = self.ln_f.forward(&x)?;
        let x = x.i((.., seq_len - 1, ..))?.contiguous()?;
        self.lm_head.forward(&x)?.to_dtype(DType::F32)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::ai_inference::candle_llm_runtime::{
        write_tachyon_tiny_mixtral_fixture, write_tachyon_tiny_mixtral_mixed_fixture,
    };
    use candle_transformers::models::llama::{Cache as DenseCache, Llama as DenseLlama};

    fn load_mixtral_config(root: &std::path::Path) -> Config {
        let raw = std::fs::read(root.join("config.json")).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        let llama_config: candle_transformers::models::llama::LlamaConfig =
            serde_json::from_value(json).unwrap();
        llama_config.into_config(false)
    }

    fn tensor_names(paths: &[std::path::PathBuf]) -> Vec<String> {
        let safetensors =
            unsafe { candle_core::safetensors::MmapedSafetensors::multi(paths) }.unwrap();
        safetensors
            .tensors()
            .into_iter()
            .map(|(name, _)| name)
            .collect()
    }

    #[test]
    fn expert_parallel_llama_matches_dense_reference_when_every_layer_is_moe() {
        let dir = tempfile::tempdir().unwrap();
        write_tachyon_tiny_mixtral_fixture(dir.path()).unwrap();
        let config = load_mixtral_config(dir.path());
        let weight_paths = vec![dir.path().join("model.safetensors")];
        let names = tensor_names(&weight_paths);

        let dense_device = Device::Cpu;
        let dense_vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&weight_paths, DType::F32, &dense_device)
        }
        .unwrap();
        // The fixture's single expert per layer makes the MoE block
        // numerically identical to a dense Llama block with the same
        // `mlp.{gate,up,down}_proj` weights (top-1 routing always selects the
        // only expert), so the dense reference is built directly against the
        // same tensor names via the standard `model.layers.{i}.mlp.*` aliases
        // the fixture writer also emits.
        let dense_model = DenseLlama::load(dense_vb, &config).unwrap();
        let mut dense_cache = DenseCache::new(false, DType::F32, &config, &dense_device).unwrap();

        let devices = vec![Device::Cpu];
        let plan = ExpertPlacementPlan::round_robin(1, devices.len());
        let ep_vb =
            unsafe { VarBuilder::from_mmaped_safetensors(&weight_paths, DType::F32, &devices[0]) }
                .unwrap();
        let ep_model = ExpertParallelLlama::load(
            ep_vb,
            &config,
            names.iter().map(String::as_str),
            &plan,
            &devices,
        )
        .unwrap();
        let mut ep_cache =
            TensorParallelCache::new(false, DType::F32, &config, &devices[0]).unwrap();

        let input = Tensor::from_vec(vec![1u32, 2, 3], (1, 3), &dense_device).unwrap();
        let dense_logits = dense_model.forward(&input, 0, &mut dense_cache).unwrap();
        let ep_logits = ep_model.forward(&input, 0, &mut ep_cache).unwrap();

        let dense_logits: Vec<f32> = dense_logits.flatten_all().unwrap().to_vec1().unwrap();
        let ep_logits: Vec<f32> = ep_logits.flatten_all().unwrap().to_vec1().unwrap();
        assert_eq!(dense_logits.len(), ep_logits.len());
        for (a, b) in dense_logits.iter().zip(ep_logits.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} != {b}");
        }
    }

    #[test]
    fn expert_parallel_llama_loads_a_mixed_dense_and_moe_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        write_tachyon_tiny_mixtral_mixed_fixture(dir.path()).unwrap();
        let config = load_mixtral_config(dir.path());
        let weight_paths = vec![dir.path().join("model.safetensors")];
        let names = tensor_names(&weight_paths);

        let devices = vec![Device::Cpu];
        let plan = ExpertPlacementPlan::round_robin(8, devices.len());
        let ep_vb =
            unsafe { VarBuilder::from_mmaped_safetensors(&weight_paths, DType::F32, &devices[0]) }
                .unwrap();
        let ep_model = ExpertParallelLlama::load(
            ep_vb,
            &config,
            names.iter().map(String::as_str),
            &plan,
            &devices,
        )
        .unwrap();
        let mut ep_cache =
            TensorParallelCache::new(false, DType::F32, &config, &devices[0]).unwrap();

        let input = Tensor::from_vec(vec![1u32, 2, 3], (1, 3), &devices[0]).unwrap();
        let logits = ep_model.forward(&input, 0, &mut ep_cache).unwrap();
        let logits: Vec<f32> = logits.flatten_all().unwrap().to_vec1().unwrap();
        assert_eq!(logits.len(), config.vocab_size);
    }

    #[test]
    fn expert_parallel_llama_decodes_a_second_token_with_shared_kv_cache() {
        let dir = tempfile::tempdir().unwrap();
        write_tachyon_tiny_mixtral_mixed_fixture(dir.path()).unwrap();
        let config = load_mixtral_config(dir.path());
        let weight_paths = vec![dir.path().join("model.safetensors")];
        let names = tensor_names(&weight_paths);

        let devices = vec![Device::Cpu];
        let plan = ExpertPlacementPlan::round_robin(8, devices.len());
        let ep_vb =
            unsafe { VarBuilder::from_mmaped_safetensors(&weight_paths, DType::F32, &devices[0]) }
                .unwrap();
        let ep_model = ExpertParallelLlama::load(
            ep_vb,
            &config,
            names.iter().map(String::as_str),
            &plan,
            &devices,
        )
        .unwrap();
        let mut ep_cache =
            TensorParallelCache::new(true, DType::F32, &config, &devices[0]).unwrap();

        let prompt = Tensor::from_vec(vec![1u32, 2, 3], (1, 3), &devices[0]).unwrap();
        ep_model.forward(&prompt, 0, &mut ep_cache).unwrap();

        let next = Tensor::from_vec(vec![0u32], (1, 1), &devices[0]).unwrap();
        let logits = ep_model.forward(&next, 3, &mut ep_cache).unwrap();
        let logits: Vec<f32> = logits.flatten_all().unwrap().to_vec1().unwrap();
        assert_eq!(logits.len(), config.vocab_size);
        assert!(logits.iter().all(|v| v.is_finite()));
    }
}
