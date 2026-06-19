//! Pipeline-parallel Llama: contiguous transformer-layer ranges ("stages")
//! placed on different devices/nodes, with activations handed off between
//! stages via `parallel::PipelineStageExecutor`/`StageTransport`.
//!
//! Each stage's local computation reuses `tensor_parallel_llama`'s
//! `TensorParallelBlock` unmodified, loaded with a single-device `devices`
//! slice (which degenerates its MLP sharding to the dense case) — there is
//! no tensor-parallelism *within* a stage here, only a contiguous run of
//! ordinary blocks. Stage 0 owns the token embedding; the last stage owns
//! the final norm and LM head, mirroring how a real deployment would only
//! materialise those weights on the nodes that need them.
//!
//! `design.md` describes reusing `ai-layer-wise-inference`'s per-layer mmap
//! streaming primitive as the per-stage executor. That primitive
//! (`ai_inference::PrefillBatch::run`) implements a placeholder forward pass
//! ("mock matmul... real implementation: call into a Candle transformer
//! block here" — see its doc comment), not real transformer math, so basing
//! a numerically-verified pipeline engine on it was not possible. This
//! module instead reuses the real, already-tested `TensorParallelBlock`
//! forward (built for Task 4) as the per-stage unit of work.

use candle_core::{DType, Device, IndexOp, Result as CandleResult, Tensor};
use candle_nn::{Module, VarBuilder};
use candle_transformers::models::llama::Config;
use candle_transformers::models::with_tracing::{
    linear_no_bias as linear, Embedding, Linear, RmsNorm,
};

use super::parallel::{PipelineStageExecutor, StageTransport};
use super::tensor_parallel_llama::{TensorParallelBlock, TensorParallelCache};

/// One pipeline stage: a contiguous range of transformer blocks, plus the
/// token embedding (stage 0 only) and/or final norm + LM head (last stage
/// only). Carries its own `Config` clone so it can build a fresh
/// `TensorParallelCache` (rotary tables, masks) without the caller having to
/// pass one in on every call — that is what makes it a self-contained
/// `PipelineStageExecutor`.
pub(crate) struct PipelineStage {
    cfg: Config,
    layer_range: (u32, u32),
    wte: Option<Embedding>,
    blocks: Vec<TensorParallelBlock>,
    head: Option<(RmsNorm, Linear)>,
    device: Device,
}

impl PipelineStage {
    /// `start`/`end` are an inclusive `[start, end]` layer range matching
    /// `parallel_topology::ParallelExecutionPlan::stage_layer_ranges`.
    pub(crate) fn load(
        vb: &VarBuilder,
        cfg: &Config,
        start: u32,
        end: u32,
        device: &Device,
        is_first_stage: bool,
        is_last_stage: bool,
    ) -> CandleResult<Self> {
        let wte = if is_first_stage {
            Some(Embedding::new(
                cfg.vocab_size,
                cfg.hidden_size,
                vb.pp("model.embed_tokens"),
            )?)
        } else {
            None
        };
        let blocks = (start..=end)
            .map(|i| {
                TensorParallelBlock::load(
                    vb.pp(format!("model.layers.{i}")),
                    cfg,
                    std::slice::from_ref(device),
                )
            })
            .collect::<CandleResult<Vec<_>>>()?;
        let head = if is_last_stage {
            let ln_f = RmsNorm::new(cfg.hidden_size, cfg.rms_norm_eps, vb.pp("model.norm"))?;
            let lm_head = if cfg.tie_word_embeddings {
                let wte_for_head =
                    Embedding::new(cfg.vocab_size, cfg.hidden_size, vb.pp("model.embed_tokens"))?;
                Linear::from_weights(wte_for_head.embeddings().clone(), None)
            } else {
                linear(cfg.hidden_size, cfg.vocab_size, vb.pp("lm_head"))?
            };
            Some((ln_f, lm_head))
        } else {
            None
        };
        Ok(Self {
            cfg: cfg.clone(),
            layer_range: (start, end),
            wte,
            blocks,
            head,
            device: device.clone(),
        })
    }

    pub(crate) fn device(&self) -> &Device {
        &self.device
    }
}

impl PipelineStageExecutor for PipelineStage {
    /// `input` is token ids (`[batch, seq]`, stage 0) or the previous
    /// stage's hidden-state activation (`[batch, seq, hidden]`, otherwise).
    /// `layer_range` is ignored in favour of the range this stage was loaded
    /// with — `run_pipeline`/`run_pipeline_microbatched` always pass the
    /// same range back, so this is purely a sanity-check assertion.
    ///
    /// This is a stateless, KV-cache-free, prefill-only forward: position 0
    /// always and a fresh cache built per call. Real decode-loop wiring
    /// (carrying a cache across calls per stage) is left for the dispatch
    /// path follow-up noted in `tasks.md`, matching Task 4's disclosed scope.
    fn run_stage(&self, layer_range: (u32, u32), input: &Tensor) -> CandleResult<Tensor> {
        debug_assert_eq!(layer_range, self.layer_range);
        let mut x = match &self.wte {
            Some(wte) => wte.forward(input)?,
            None => input.clone(),
        };
        let dtype = x.dtype();
        let mut cache = TensorParallelCache::new(false, dtype, &self.cfg, &self.device)?;
        for (offset, block) in self.blocks.iter().enumerate() {
            let block_idx = self.layer_range.0 as usize + offset;
            x = block.forward(&x, 0, block_idx, &mut cache)?;
        }
        if let Some((ln_f, lm_head)) = &self.head {
            let x = ln_f.forward(&x)?;
            let x = if x.dims().len() == 3 {
                let seq_len = x.dim(1)?;
                x.i((.., seq_len - 1, ..))?.contiguous()?
            } else {
                x
            };
            lm_head.forward(&x)?.to_dtype(DType::F32)
        } else {
            Ok(x)
        }
    }
}

/// Owns every stage of a pipeline-parallel deployment and drives prefill
/// through all of them via `parallel::run_pipeline`-style sequencing. A real
/// cross-node deployment runs one `PipelineStage` per node and hands
/// activations off with `parallel::TcpStageTransport`; this struct is the
/// single-process reference composition used to prove the per-stage math
/// and the transport boundary are both correct.
pub(crate) struct PipelineParallelLlama {
    pub(crate) stages: Vec<PipelineStage>,
}

impl PipelineParallelLlama {
    /// `stage_layer_ranges` must be contiguous, cover `[0, cfg.num_hidden_layers)`
    /// exactly once, and have one entry per `devices[i]` — the same shape
    /// `parallel_topology::validate_plan_shape` already enforces for a
    /// pipeline-parallel plan at `apply-model-deployment` time.
    pub(crate) fn load(
        vb: VarBuilder,
        cfg: &Config,
        stage_layer_ranges: &[(u32, u32)],
        devices: &[Device],
    ) -> CandleResult<Self> {
        let n = stage_layer_ranges.len();
        let stages = stage_layer_ranges
            .iter()
            .zip(devices.iter())
            .enumerate()
            .map(|(i, (&(start, end), device))| {
                PipelineStage::load(&vb, cfg, start, end, device, i == 0, i == n - 1)
            })
            .collect::<CandleResult<Vec<_>>>()?;
        Ok(Self { stages })
    }

    /// Runs a single forward pass (prefill) for `tokens` (`[batch, seq]`)
    /// through every stage in order, transporting the activation between
    /// stages with `transports[i]` after stage `i` (an `InProcessTransport`
    /// for same-process multi-device, a `TcpStageTransport` for real
    /// cross-node hand-off — both implement the same `StageTransport` trait,
    /// so callers choose by construction, not by a branch in this method).
    pub(crate) fn forward(
        &self,
        tokens: &Tensor,
        transports: &[Box<dyn StageTransport>],
    ) -> CandleResult<Tensor> {
        let mut activation = tokens.clone();
        for (i, stage) in self.stages.iter().enumerate() {
            activation = stage.run_stage(stage.layer_range, &activation)?;
            if let Some(transport) = transports.get(i) {
                activation = transport.send(activation)?;
            }
        }
        Ok(activation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai_inference::candle_llm_runtime::write_tachyon_tiny_fixture;
    use candle_transformers::models::llama::{
        Cache as DenseCache, Llama as DenseLlama, LlamaConfig,
    };

    fn load_config(root: &std::path::Path) -> Config {
        let raw = std::fs::read(root.join("config.json")).unwrap();
        let llama_config: LlamaConfig = serde_json::from_slice(&raw).unwrap();
        llama_config.into_config(false)
    }

    fn build_model(
        root: &std::path::Path,
        cfg: &Config,
        stage_ranges: &[(u32, u32)],
    ) -> PipelineParallelLlama {
        let devices = vec![Device::Cpu; stage_ranges.len()];
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(
                &[root.join("model.safetensors")],
                DType::F32,
                &Device::Cpu,
            )
            .unwrap()
        };
        PipelineParallelLlama::load(vb, cfg, stage_ranges, &devices).unwrap()
    }

    #[test]
    fn pipeline_parallel_llama_matches_dense_reference_on_a_real_checkpoint() {
        let tmp = tempfile::tempdir().unwrap();
        write_tachyon_fixture_or_panic(tmp.path());
        let cfg = load_config(tmp.path());

        let dense_vb = unsafe {
            VarBuilder::from_mmaped_safetensors(
                &[tmp.path().join("model.safetensors")],
                DType::F32,
                &Device::Cpu,
            )
            .unwrap()
        };
        let dense_model = DenseLlama::load(dense_vb, &cfg).unwrap();
        let mut dense_cache = DenseCache::new(false, DType::F32, &cfg, &Device::Cpu).unwrap();

        let mid = (cfg.num_hidden_layers as u32) / 2;
        let stage_ranges = vec![(0, mid - 1), (mid, cfg.num_hidden_layers as u32 - 1)];
        let pipeline_model = build_model(tmp.path(), &cfg, &stage_ranges);

        let tokens = Tensor::from_vec(vec![1u32, 2, 1], (1, 3), &Device::Cpu).unwrap();
        let dense_logits = dense_model.forward(&tokens, 0, &mut dense_cache).unwrap();

        let transports: Vec<Box<dyn StageTransport>> = vec![Box::new(
            crate::ai_inference::parallel::InProcessTransport {
                next_device: Device::Cpu,
            },
        )];
        let pipeline_logits = pipeline_model.forward(&tokens, &transports).unwrap();

        let dense: Vec<f32> = dense_logits.flatten_all().unwrap().to_vec1().unwrap();
        let pipeline: Vec<f32> = pipeline_logits.flatten_all().unwrap().to_vec1().unwrap();
        assert_eq!(dense.len(), pipeline.len());
        for (a, b) in dense.iter().zip(pipeline.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} != {b}");
        }
    }

    #[test]
    fn pipeline_parallel_llama_hands_off_activations_over_a_real_tcp_socket() {
        let tmp = tempfile::tempdir().unwrap();
        write_tachyon_fixture_or_panic(tmp.path());
        let cfg = load_config(tmp.path());

        let mid = (cfg.num_hidden_layers as u32) / 2;
        let stage_ranges = vec![(0, mid - 1), (mid, cfg.num_hidden_layers as u32 - 1)];

        // In-process reference (no network).
        let reference_model = build_model(tmp.path(), &cfg, &stage_ranges);
        let tokens = Tensor::from_vec(vec![1u32, 2, 1], (1, 3), &Device::Cpu).unwrap();
        let in_process_transports: Vec<Box<dyn StageTransport>> = vec![Box::new(
            crate::ai_inference::parallel::InProcessTransport {
                next_device: Device::Cpu,
            },
        )];
        let reference_logits = reference_model
            .forward(&tokens, &in_process_transports)
            .unwrap();

        // Same forward, but stage 0 -> stage 1's hand-off goes over a real
        // TCP socket: a background thread runs stage 1's forward when the
        // activation arrives, and sends the result back.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stage1_model = build_model(tmp.path(), &cfg, &stage_ranges);
        let server = std::thread::spawn(move || {
            let stage1 = &stage1_model.stages[1];
            let layer_range = stage1.layer_range;
            crate::ai_inference::parallel::TcpStageTransport::serve_one(
                &listener,
                &Device::Cpu,
                |x| stage1.run_stage(layer_range, &x),
            )
            .unwrap();
        });

        let stage0_model = build_model(tmp.path(), &cfg, &stage_ranges);
        let stage0 = &stage0_model.stages[0];
        let stage0_only_logits = stage0.run_stage(stage0.layer_range, &tokens).unwrap();
        let tcp_transport = crate::ai_inference::parallel::TcpStageTransport::new(addr);
        let networked_logits = tcp_transport.send(stage0_only_logits).unwrap();
        server.join().unwrap();

        let reference: Vec<f32> = reference_logits.flatten_all().unwrap().to_vec1().unwrap();
        let networked: Vec<f32> = networked_logits.flatten_all().unwrap().to_vec1().unwrap();
        assert_eq!(reference.len(), networked.len());
        for (a, b) in reference.iter().zip(networked.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} != {b}");
        }
    }

    fn write_tachyon_fixture_or_panic(root: &std::path::Path) {
        write_tachyon_tiny_fixture(root).unwrap();
    }
}
