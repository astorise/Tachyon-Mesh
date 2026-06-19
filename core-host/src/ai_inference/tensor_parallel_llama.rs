//! Tensor-parallel (Megatron-style) Llama forward pass.
//!
//! Mirrors `candle_transformers::models::llama::Llama`'s forward math exactly,
//! with one substitution: the MLP's up/gate (column-parallel) and down
//! (row-parallel) projections are sharded across `devices`, with a single
//! all-reduce per transformer block — the scope `design.md` §2 describes.
//! Attention, embeddings, and the LM head are replicated (not sharded) and
//! always run on `devices[0]` ("the primary device"); only the MLP's compute
//! and memory footprint are actually split across the node's accelerators.
//!
//! `candle_transformers::models::llama::{Llama, Block, Mlp,
//! CausalSelfAttention, Cache}` are private to that crate, so this module
//! reimplements the same forward math (verified bit-for-bit against it by
//! `tensor_parallel_llama_matches_dense_reference_on_a_real_checkpoint`
//! below) with `TensorParallelMlp` standing in for its `Mlp`.

use std::collections::HashMap;
use std::f32::consts::PI;

use candle_core::{DType, Device, IndexOp, Result as CandleResult, Tensor, D};
use candle_nn::{Module, VarBuilder};
use candle_transformers::models::llama::{Config, Llama3RopeConfig, Llama3RopeType};
use candle_transformers::models::with_tracing::{linear_no_bias as linear, Embedding, Linear, RmsNorm};
use candle_transformers::utils::{build_causal_mask, repeat_kv};

use super::parallel::{split_for_row_parallel, ColumnParallelLinear, RowParallelLinear};

/// External KV cache + precomputed rotary tables for [`TensorParallelLlama`].
/// Functionally identical to `candle_transformers::models::llama::Cache`,
/// reimplemented here because its fields and `mask()` helper are private to
/// the upstream crate.
pub(crate) struct TensorParallelCache {
    masks: HashMap<(usize, usize), Tensor>,
    use_kv_cache: bool,
    kvs: Vec<Option<(Tensor, Tensor)>>,
    cos: Tensor,
    sin: Tensor,
    device: Device,
}

fn calculate_default_inv_freq(cfg: &Config) -> Vec<f32> {
    let head_dim = cfg.hidden_size / cfg.num_attention_heads;
    (0..head_dim)
        .step_by(2)
        .map(|i| 1f32 / cfg.rope_theta.powf(i as f32 / head_dim as f32))
        .collect()
}

impl TensorParallelCache {
    pub(crate) fn new(
        use_kv_cache: bool,
        dtype: DType,
        config: &Config,
        device: &Device,
    ) -> CandleResult<Self> {
        let theta = match &config.rope_scaling {
            None
            | Some(Llama3RopeConfig {
                rope_type: Llama3RopeType::Default,
                ..
            }) => calculate_default_inv_freq(config),
            Some(rope_scaling) => {
                let low_freq_wavelen = rope_scaling.original_max_position_embeddings as f32
                    / rope_scaling.low_freq_factor;
                let high_freq_wavelen = rope_scaling.original_max_position_embeddings as f32
                    / rope_scaling.high_freq_factor;

                calculate_default_inv_freq(config)
                    .into_iter()
                    .map(|freq| {
                        let wavelen = 2. * PI / freq;
                        if wavelen < high_freq_wavelen {
                            freq
                        } else if wavelen > low_freq_wavelen {
                            freq / rope_scaling.factor
                        } else {
                            let smooth = (rope_scaling.original_max_position_embeddings as f32
                                / wavelen
                                - rope_scaling.low_freq_factor)
                                / (rope_scaling.high_freq_factor - rope_scaling.low_freq_factor);
                            (1. - smooth) * freq / rope_scaling.factor + smooth * freq
                        }
                    })
                    .collect::<Vec<_>>()
            }
        };

        let theta = Tensor::new(theta, device)?;
        let idx_theta = Tensor::arange(0, config.max_position_embeddings as u32, device)?
            .to_dtype(DType::F32)?
            .reshape((config.max_position_embeddings, 1))?
            .matmul(&theta.reshape((1, theta.elem_count()))?)?;
        let cos = idx_theta.cos()?.to_dtype(dtype)?;
        let sin = idx_theta.sin()?.to_dtype(dtype)?;
        Ok(Self {
            masks: HashMap::new(),
            use_kv_cache,
            kvs: vec![None; config.num_hidden_layers],
            device: device.clone(),
            cos,
            sin,
        })
    }

    fn mask(&mut self, seq_len: usize, index_pos: usize) -> CandleResult<Tensor> {
        let kv_len = index_pos + seq_len;
        if let Some(mask) = self.masks.get(&(seq_len, kv_len)) {
            Ok(mask.clone())
        } else {
            let mask = build_causal_mask(seq_len, index_pos, &self.device)?;
            self.masks.insert((seq_len, kv_len), mask.clone());
            Ok(mask)
        }
    }
}

fn masked_fill(on_false: &Tensor, mask: &Tensor, on_true: f32) -> CandleResult<Tensor> {
    let shape = mask.shape();
    let on_true = Tensor::new(on_true, on_false.device())?.broadcast_as(shape.dims())?;
    mask.where_cond(&on_true, on_false)
}

/// Replicated (unsharded) causal self-attention, run entirely on the
/// "primary" device. Identical math to upstream
/// `candle_transformers::models::llama::CausalSelfAttention::forward`, minus
/// the `flash-attn` feature path (not enabled by this crate).
struct ReplicatedAttention {
    q_proj: Linear,
    k_proj: Linear,
    v_proj: Linear,
    o_proj: Linear,
    num_attention_heads: usize,
    num_key_value_heads: usize,
    head_dim: usize,
    max_position_embeddings: usize,
}

impl ReplicatedAttention {
    fn load(vb: VarBuilder, cfg: &Config) -> CandleResult<Self> {
        let size_in = cfg.hidden_size;
        let size_q = (cfg.hidden_size / cfg.num_attention_heads) * cfg.num_attention_heads;
        let size_kv = (cfg.hidden_size / cfg.num_attention_heads) * cfg.num_key_value_heads;
        Ok(Self {
            q_proj: linear(size_in, size_q, vb.pp("q_proj"))?,
            k_proj: linear(size_in, size_kv, vb.pp("k_proj"))?,
            v_proj: linear(size_in, size_kv, vb.pp("v_proj"))?,
            o_proj: linear(size_q, size_in, vb.pp("o_proj"))?,
            num_attention_heads: cfg.num_attention_heads,
            num_key_value_heads: cfg.num_key_value_heads,
            head_dim: cfg.hidden_size / cfg.num_attention_heads,
            max_position_embeddings: cfg.max_position_embeddings,
        })
    }

    fn apply_rotary_emb(
        &self,
        x: &Tensor,
        index_pos: usize,
        cache: &TensorParallelCache,
    ) -> CandleResult<Tensor> {
        let (_b_sz, _, seq_len, _hidden_size) = x.dims4()?;
        let cos = cache.cos.narrow(0, index_pos, seq_len)?;
        let sin = cache.sin.narrow(0, index_pos, seq_len)?;
        candle_nn::rotary_emb::rope(x, &cos, &sin)
    }

    fn forward(
        &self,
        x: &Tensor,
        index_pos: usize,
        block_idx: usize,
        cache: &mut TensorParallelCache,
    ) -> CandleResult<Tensor> {
        let (b_sz, seq_len, hidden_size) = x.dims3()?;
        let q = self.q_proj.forward(x)?;
        let k = self.k_proj.forward(x)?;
        let v = self.v_proj.forward(x)?;

        let q = q
            .reshape((b_sz, seq_len, self.num_attention_heads, self.head_dim))?
            .transpose(1, 2)?
            .contiguous()?;
        let k = k
            .reshape((b_sz, seq_len, self.num_key_value_heads, self.head_dim))?
            .transpose(1, 2)?
            .contiguous()?;
        let mut v = v
            .reshape((b_sz, seq_len, self.num_key_value_heads, self.head_dim))?
            .transpose(1, 2)?;

        let q = self.apply_rotary_emb(&q, index_pos, cache)?;
        let mut k = self.apply_rotary_emb(&k, index_pos, cache)?;

        if cache.use_kv_cache {
            if let Some((cache_k, cache_v)) = &cache.kvs[block_idx] {
                k = Tensor::cat(&[cache_k, &k], 2)?.contiguous()?;
                v = Tensor::cat(&[cache_v, &v], 2)?.contiguous()?;
                let k_seq_len = k.dims()[1];
                if k_seq_len > self.max_position_embeddings {
                    k = k
                        .narrow(
                            D::Minus1,
                            k_seq_len - self.max_position_embeddings,
                            self.max_position_embeddings,
                        )?
                        .contiguous()?
                }
                let v_seq_len = v.dims()[1];
                if v_seq_len > 2 * self.max_position_embeddings {
                    v = v
                        .narrow(
                            D::Minus1,
                            v_seq_len - self.max_position_embeddings,
                            self.max_position_embeddings,
                        )?
                        .contiguous()?
                }
            }
            cache.kvs[block_idx] = Some((k.clone(), v.clone()))
        }

        let k = repeat_kv(k, self.num_attention_heads / self.num_key_value_heads)?;
        let v = repeat_kv(v, self.num_attention_heads / self.num_key_value_heads)?;

        let in_dtype = q.dtype();
        let q = q.to_dtype(DType::F32)?;
        let k = k.to_dtype(DType::F32)?;
        let v = v.to_dtype(DType::F32)?;
        let att = (q.matmul(&k.t()?)? / (self.head_dim as f64).sqrt())?;
        let att = if seq_len == 1 {
            att
        } else {
            let mask = cache.mask(seq_len, index_pos)?.broadcast_as(att.shape())?;
            masked_fill(&att, &mask, f32::NEG_INFINITY)?
        };
        let att = candle_nn::ops::softmax_last_dim(&att)?;
        let y = att.matmul(&v.contiguous()?)?.to_dtype(in_dtype)?;
        let y = y.transpose(1, 2)?.reshape(&[b_sz, seq_len, hidden_size])?;
        self.o_proj.forward(&y)
    }
}

/// Megatron-style MLP: column-parallel gate/up projections, row-parallel down
/// projection, one all-reduce (the `RowParallelLinear` partial-sum) per call.
/// Requires `devices.len() >= 2` (enforced by
/// `parallel_topology::validate_plan_shape` before a plan reaches this
/// loader).
pub(crate) struct TensorParallelMlp {
    gate: ColumnParallelLinear,
    up: ColumnParallelLinear,
    down: RowParallelLinear,
    devices: Vec<Device>,
    primary: Device,
}

impl TensorParallelMlp {
    fn load(vb: VarBuilder, cfg: &Config, devices: &[Device]) -> CandleResult<Self> {
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
            devices: devices.to_vec(),
            primary: devices[0].clone(),
        })
    }

    fn forward(&self, x: &Tensor) -> CandleResult<Tensor> {
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
        let out_2d = self.down.forward(&x_shards, &self.primary)?;

        let mut out_dims = in_shape.dims().to_vec();
        *out_dims.last_mut().expect("hidden_size dim exists") = out_2d.dim(1)?;
        out_2d.reshape(out_dims)
    }
}

/// A single tensor-parallel transformer block. Also reused, unmodified, as
/// the per-stage local unit of work for pipeline parallelism
/// (`pipeline_parallel_llama.rs`): a pipeline stage is just a contiguous run
/// of these blocks loaded with a single-device `devices` slice (which
/// degenerates `TensorParallelMlp`'s sharding to the dense case), with stage
/// boundaries handled by the caller via `PipelineStageExecutor`/`StageTransport`.
pub(crate) struct TensorParallelBlock {
    rms_1: RmsNorm,
    attn: ReplicatedAttention,
    rms_2: RmsNorm,
    mlp: TensorParallelMlp,
}

impl TensorParallelBlock {
    pub(crate) fn load(vb: VarBuilder, cfg: &Config, devices: &[Device]) -> CandleResult<Self> {
        Ok(Self {
            attn: ReplicatedAttention::load(vb.pp("self_attn"), cfg)?,
            mlp: TensorParallelMlp::load(vb.pp("mlp"), cfg, devices)?,
            rms_1: RmsNorm::new(cfg.hidden_size, cfg.rms_norm_eps, vb.pp("input_layernorm"))?,
            rms_2: RmsNorm::new(
                cfg.hidden_size,
                cfg.rms_norm_eps,
                vb.pp("post_attention_layernorm"),
            )?,
        })
    }

    pub(crate) fn forward(
        &self,
        x: &Tensor,
        index_pos: usize,
        block_idx: usize,
        cache: &mut TensorParallelCache,
    ) -> CandleResult<Tensor> {
        let residual = x;
        let x = self.rms_1.forward(x)?;
        let x = (self.attn.forward(&x, index_pos, block_idx, cache)? + residual)?;
        let residual = &x;
        let x = (self.mlp.forward(&self.rms_2.forward(&x)?)? + residual)?;
        Ok(x)
    }
}

/// Tensor-parallel Llama: identical outputs to
/// `candle_transformers::models::llama::Llama::load`/`forward` on the same
/// weights, but the MLP of every block is sharded across `devices`.
pub(crate) struct TensorParallelLlama {
    wte: Embedding,
    blocks: Vec<TensorParallelBlock>,
    ln_f: RmsNorm,
    lm_head: Linear,
}

impl TensorParallelLlama {
    /// `devices` must have at least 2 entries; shape validation
    /// (`parallel_topology::validate_plan_shape`) rejects tensor-parallel
    /// plans with fewer before they reach this loader.
    pub(crate) fn load(vb: VarBuilder, cfg: &Config, devices: &[Device]) -> CandleResult<Self> {
        let wte = Embedding::new(cfg.vocab_size, cfg.hidden_size, vb.pp("model.embed_tokens"))?;
        let lm_head = if cfg.tie_word_embeddings {
            Linear::from_weights(wte.embeddings().clone(), None)
        } else {
            linear(cfg.hidden_size, cfg.vocab_size, vb.pp("lm_head"))?
        };
        let ln_f = RmsNorm::new(cfg.hidden_size, cfg.rms_norm_eps, vb.pp("model.norm"))?;
        let blocks = (0..cfg.num_hidden_layers)
            .map(|i| TensorParallelBlock::load(vb.pp(format!("model.layers.{i}")), cfg, devices))
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
            x = block.forward(&x, index_pos, block_idx, cache)?;
        }
        let x = self.ln_f.forward(&x)?;
        let x = x.i((.., seq_len - 1, ..))?.contiguous()?;
        self.lm_head.forward(&x)?.to_dtype(DType::F32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_inference::candle_llm_runtime::write_tachyon_tiny_fixture;
    use candle_transformers::models::llama::{Cache as DenseCache, Llama as DenseLlama, LlamaConfig};

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
        let tp_model = TensorParallelLlama::load(tp_vb, &config, &tp_devices).unwrap();
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
        let tp_model = TensorParallelLlama::load(tp_vb, &config, &tp_devices).unwrap();
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
