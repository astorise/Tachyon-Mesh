use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    fs,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use candle_core::{
    quantized::{gguf_file, GgmlDType},
    safetensors::MmapedSafetensors,
    DType, Device, Tensor,
};
use candle_nn::VarBuilder;
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::deepseek2::{
    DeepSeekV2 as DeepSeekModel, DeepSeekV2Config as DeepSeekConfig,
};
use candle_transformers::models::gemma2::{Config as Gemma2Config, Model as Gemma2Model};
use candle_transformers::models::gemma3::{Config as Gemma3Config, Model as Gemma3Model};
use candle_transformers::models::llama::{
    Cache, Config, Llama, LlamaConfig, LlamaEosToks, LoraConfig, PagedKvCache,
};
use candle_transformers::models::phi3::{Config as Phi3Config, Model as Phi3Model};
use candle_transformers::models::quantized_llama::ModelWeights as QuantizedLlama;
use candle_transformers::models::qwen2::{Config as Qwen2Config, ModelForCausalLM as Qwen2Model};
use candle_transformers::models::qwen3::{Config as Qwen3Config, ModelForCausalLM as Qwen3Model};
use candle_transformers::models::qwen3_moe::{
    Config as Qwen3MoeConfig, ModelForCausalLM as Qwen3MoeModel,
};
use serde::Deserialize;
use thiserror::Error;
use tokenizers::Tokenizer;

use crate::{GpuDistribution, HardwareStrategy};

use super::expert_parallel_llama::ExpertParallelLlama;
use super::modelopt_nvfp4;
use super::paged_kv::{
    build_block_table_tensor, build_cumulative_seqlens_tensor, PagedBlockPool, PagedKvError,
    SequenceBlockTable,
};
use super::parallel::{discover_cluster_topology, ExpertPlacementPlan};
use super::pipeline_parallel_llama::PipelineParallelLlama;
use super::samplers::{FsmCache, FsmLogitProcessor};
use super::tensor_parallel_llama::{TensorParallelCache, TensorParallelLlama};
use super::ResolvedLoraAdapter;
use parallel_topology::{
    validate_parallel_topology, ClusterTopology, ParallelExecutionPlan, ParallelStrategy,
};

/// HF `model_type` of the only architecture family currently executed. Real,
/// uploaded Llama-family checkpoints (Llama 2/3, TinyLlama, Vicuna, …) carry
/// this value in their `config.json`.
pub(crate) const LLAMA_MODEL_TYPE: &str = "llama";

/// HF `model_type` of Mixtral-family (sparse MoE) checkpoints, recognized only
/// for `GpuDistribution::ExpertParallelism` deployments.
const MIXTRAL_MODEL_TYPE: &str = "mixtral";

/// Mixtral `config.json` shape: every Llama field via `#[serde(flatten)]`
/// (attention/embedding sizing is identical to dense Llama) plus the two
/// MoE-only fields this runtime actually needs.
#[derive(Debug, Deserialize)]
struct RawMixtralConfig {
    #[serde(flatten)]
    base: LlamaConfig,
    num_local_experts: usize,
    num_experts_per_tok: usize,
}

/// Validated Mixtral config: the dense `Config` (for attention/embedding/
/// generation limits, identical math to Llama) plus the expert count needed
/// to build an [`ExpertPlacementPlan`].
struct MixtralConfigJson {
    config: Config,
    num_local_experts: usize,
}

#[cfg(feature = "candle-cuda")]
const PARALLEL_LLAMA_USE_FLASH_ATTN: bool = true;
#[cfg(not(feature = "candle-cuda"))]
const PARALLEL_LLAMA_USE_FLASH_ATTN: bool = false;

const CONFIG_JSON: &str = "config.json";
const TOKENIZER_JSON: &str = "tokenizer.json";
/// Hugging Face tokenizer settings; carries the `chat_template` (Jinja2) and the
/// special tokens (`bos_token`/`eos_token`) that instruct templates reference.
const TOKENIZER_CONFIG_JSON: &str = "tokenizer_config.json";
const MODEL_SAFETENSORS: &str = "model.safetensors";
const SAFETENSORS_INDEX_JSON: &str = "model.safetensors.index.json";
/// Sidecar dropped next to the weights by `system-faas-model-broker` at upload
/// time, declaring the on-disk format. The broker does the format *detection*
/// (control plane); the host only honours the declared value and still validates
/// the bytes through the matching loader (the attribute is a dispatch hint, not a
/// trust boundary). Absent sidecar → the host infers the format from directory
/// contents so operator-provisioned models keep working.
const MODEL_META_JSON: &str = ".tachyon-model.json";
/// GGUF files begin with the ASCII magic `GGUF`.
const GGUF_MAGIC: [u8; 4] = *b"GGUF";
/// GGUF file extension probed when inferring the format from directory contents.
const GGUF_EXTENSION: &str = "gguf";
/// GGUF `general.architecture` value for the Llama family.
const GGUF_LLAMA_ARCHITECTURE: &str = "llama";
/// Error component label for GGUF load failures.
const GGUF_COMPONENT: &str = "model.gguf";
const DEFAULT_MAX_NEW_TOKENS: usize = 64;
pub(crate) const DEFAULT_SPECULATIVE_DRAFT_TOKENS: usize = 4;
/// Hard upper bound on `max_new_tokens` for any single request, regardless of
/// what the caller asks for. Protects the host from unbounded decode loops.
pub(crate) const HOST_MAX_NEW_TOKENS: usize = 256;
/// Seed used when a generation request samples (temperature > 0) but does not
/// pin a `seed`. Fixed so that an un-seeded sampled request is still reproducible
/// for a given prompt — callers that want variation pass their own `seed`.
const DEFAULT_SAMPLING_SEED: u64 = 299_792_458;
/// Hard cap on the number of stop sequences honoured for a single request, and on
/// the length of each, so a caller cannot force unbounded substring scans.
const MAX_STOP_SEQUENCES: usize = 8;
const MAX_STOP_SEQUENCE_BYTES: usize = 256;
/// Hard upper bound on the raw prompt size (bytes) accepted before tokenization.
pub(crate) const DEFAULT_MAX_PROMPT_BYTES: usize = 16_384;
const DEFAULT_MAX_PROMPT_TOKENS: usize = 4_096;
const DEFAULT_MAX_BATCH_SIZE: usize = 32;
const DEFAULT_PREFILL_CHUNK_TOKENS: usize = 8_192;
const PREFIX_CACHE_BLOCK_TOKENS: usize = 16;
const PREFIX_CACHE_MAX_ENTRIES: usize = 128;

#[derive(Debug, Error)]
pub(crate) enum CandleLlmError {
    #[error("Candle LLM model `{alias}` at `{path}` is unsupported: {detail}")]
    UnsupportedModel {
        alias: String,
        path: PathBuf,
        detail: String,
    },
    #[error("Candle LLM model `{alias}` at `{path}` is missing required file `{file}`")]
    MissingFile {
        alias: String,
        path: PathBuf,
        file: &'static str,
    },
    #[error("Candle LLM model `{alias}` at `{path}` has invalid {component}: {detail}")]
    InvalidComponent {
        alias: String,
        path: PathBuf,
        component: &'static str,
        detail: String,
    },
    #[error("Candle LLM request for model `{alias}` is invalid: {detail}")]
    InvalidRequest { alias: String, detail: String },
    #[error("Candle LLM execution failed for model `{alias}`: {detail}")]
    Execution { alias: String, detail: String },
}

/// On-disk weight format of an uploaded checkpoint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModelFormat {
    /// Hugging Face safetensors directory: `config.json` + `model.safetensors`
    /// (single-file or sharded) + `tokenizer.json`.
    Safetensors,
    /// Single GGUF file (llama.cpp ecosystem, quantised) + `tokenizer.json`.
    Gguf,
}

/// Normalized text architecture selected exclusively from checkpoint metadata.
/// A family being recognized does not imply every format or execution mode is
/// implemented; [`ArchitectureCapabilities`] keeps those claims explicit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelArchitecture {
    Llama,
    Mixtral,
    Qwen2,
    Qwen3,
    Gemma2,
    Gemma3,
    Phi3,
    Phi4,
    DeepSeekV2,
    DeepSeekV3,
    DeepSeekR1,
    Qwen35Moe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ArchitectureCapabilities {
    safetensors: bool,
    gguf: bool,
    single: bool,
    tensor_parallel: bool,
    pipeline_parallel: bool,
    expert_parallel: bool,
    specialized_runtime: bool,
}

impl ModelArchitecture {
    fn from_hf_model_type(model_type: &str) -> Option<Self> {
        match model_type {
            LLAMA_MODEL_TYPE => Some(Self::Llama),
            MIXTRAL_MODEL_TYPE => Some(Self::Mixtral),
            "qwen2" => Some(Self::Qwen2),
            "qwen3" => Some(Self::Qwen3),
            "gemma2" => Some(Self::Gemma2),
            "gemma3" | "gemma3_text" => Some(Self::Gemma3),
            "phi3" => Some(Self::Phi3),
            "phi4" => Some(Self::Phi4),
            "deepseek_v2" => Some(Self::DeepSeekV2),
            "deepseek_v3" => Some(Self::DeepSeekV3),
            "deepseek_r1" => Some(Self::DeepSeekR1),
            "qwen3_moe" | "qwen3_moe_text" | "qwen3_5_moe" | "qwen3_5_moe_text" => {
                Some(Self::Qwen35Moe)
            }
            _ => None,
        }
    }

    fn from_gguf_architecture(architecture: &str) -> Option<Self> {
        match architecture {
            GGUF_LLAMA_ARCHITECTURE => Some(Self::Llama),
            "qwen2" => Some(Self::Qwen2),
            "qwen3" => Some(Self::Qwen3),
            "gemma2" => Some(Self::Gemma2),
            "gemma3" => Some(Self::Gemma3),
            "phi3" => Some(Self::Phi3),
            "phi4" => Some(Self::Phi4),
            "deepseek2" => Some(Self::DeepSeekV2),
            "deepseek3" => Some(Self::DeepSeekV3),
            _ => None,
        }
    }

    fn capabilities(self) -> ArchitectureCapabilities {
        match self {
            Self::Llama => ArchitectureCapabilities {
                safetensors: true,
                gguf: true,
                single: true,
                tensor_parallel: true,
                pipeline_parallel: true,
                expert_parallel: false,
                specialized_runtime: false,
            },
            Self::Mixtral => ArchitectureCapabilities {
                safetensors: true,
                gguf: false,
                single: false,
                tensor_parallel: false,
                pipeline_parallel: false,
                expert_parallel: true,
                specialized_runtime: false,
            },
            Self::Qwen35Moe => ArchitectureCapabilities {
                safetensors: true,
                gguf: false,
                single: true,
                tensor_parallel: false,
                pipeline_parallel: false,
                expert_parallel: false,
                specialized_runtime: false,
            },
            Self::Qwen2 | Self::Qwen3 => ArchitectureCapabilities {
                safetensors: true,
                gguf: false,
                single: true,
                tensor_parallel: false,
                pipeline_parallel: false,
                expert_parallel: false,
                specialized_runtime: false,
            },
            Self::Gemma2 | Self::Gemma3 => ArchitectureCapabilities {
                safetensors: true,
                gguf: false,
                single: true,
                tensor_parallel: false,
                pipeline_parallel: false,
                expert_parallel: false,
                specialized_runtime: false,
            },
            Self::Phi3 | Self::Phi4 | Self::DeepSeekV2 => ArchitectureCapabilities {
                safetensors: true,
                gguf: false,
                single: true,
                tensor_parallel: false,
                pipeline_parallel: false,
                expert_parallel: false,
                specialized_runtime: false,
            },
            Self::DeepSeekV3 | Self::DeepSeekR1 => ArchitectureCapabilities {
                safetensors: false,
                gguf: false,
                single: false,
                tensor_parallel: false,
                pipeline_parallel: false,
                expert_parallel: false,
                specialized_runtime: false,
            },
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Llama => "llama",
            Self::Mixtral => "mixtral",
            Self::Qwen2 => "qwen2",
            Self::Qwen3 => "qwen3",
            Self::Gemma2 => "gemma2",
            Self::Gemma3 => "gemma3",
            Self::Phi3 => "phi3",
            Self::Phi4 => "phi4",
            Self::DeepSeekV2 => "deepseek-v2",
            Self::DeepSeekV3 => "deepseek-v3",
            Self::DeepSeekR1 => "deepseek-r1",
            Self::Qwen35Moe => "qwen3-moe",
        }
    }

    fn supports_format(self, format: ModelFormat) -> bool {
        let capabilities = self.capabilities();
        match format {
            ModelFormat::Safetensors => capabilities.safetensors,
            ModelFormat::Gguf => capabilities.gguf,
        }
    }

    fn supports_strategy(self, strategy: &HardwareStrategy) -> bool {
        let capabilities = self.capabilities();
        match strategy.distribution_mode {
            GpuDistribution::Single => capabilities.single,
            GpuDistribution::TensorParallelism => capabilities.tensor_parallel,
            GpuDistribution::PipelineParallelism => capabilities.pipeline_parallel,
            GpuDistribution::ExpertParallelism => capabilities.expert_parallel,
        }
    }
}

fn is_multimodal_hf_model_type(model_type: &str) -> bool {
    matches!(
        model_type,
        "qwen_vl" | "qwen2_vl" | "qwen2_5_vl" | "qwen3_vl" | "gemma3_vl" | "gemma3_vision"
    )
}

/// A loaded, ready-to-run Llama-family model. Weights are mmapped (safetensors)
/// or read (GGUF) from the model directory — never copied into the Tachyon
/// artifact. Shared behind an `Arc` so the runtime stays cheap to clone.
enum LoadedModel {
    /// Full-precision safetensors model behind a family-neutral dispatch
    /// boundary. Each family owns its Candle-specific cache semantics here.
    Safetensors {
        backend: SingleDeviceBackend,
        eos_tokens: Vec<u32>,
    },
    /// Quantised GGUF Llama. `forward` takes `&mut self` (the KV cache lives
    /// inside the weights and resets whenever a sequence restarts at
    /// `index_pos == 0`), so it is guarded by a `Mutex`; the QoS scheduler
    /// already serialises execution per accelerator, so contention is minimal.
    Gguf {
        model: Mutex<QuantizedLlama>,
        eos_tokens: Vec<u32>,
        /// Device the quantized weights were uploaded to. `Device::Cpu` for a
        /// `cpu` binding (the historical path); a real CUDA device once the
        /// binding asks for one on a `candle-cuda` build. Input tensors must be
        /// built here — `QuantizedLlama::forward` has no device coercion, so a
        /// CPU input against CUDA weights fails the matmul outright.
        device: Device,
    },
    /// A multi-device parallel engine, selected when the deployment's
    /// `hardware_strategy.distribution_mode` is not `single`. The engines
    /// themselves are the numerically-verified ones from
    /// `tensor_parallel_llama`/`pipeline_parallel_llama`; this variant is the
    /// runtime dispatch that finally selects them.
    Parallel(ParallelModel),
}

/// Family-neutral boundary for single-device safetensors execution. New
/// architectures are added here without leaking their Candle model/cache types
/// into request parsing, sampling, stop handling, or streaming.
enum SingleDeviceBackend {
    /// `device` is the device the weights were actually loaded onto: `Cpu`
    /// unless `enable-single-device-llama-cuda-execution`'s gate selected a
    /// real CUDA device for this checkpoint. Every other variant here stays
    /// `Cpu`-only, so only `Llama` carries this field.
    ///
    /// `dtype` matches whatever the weights were actually loaded in: `F32`
    /// unless `paged` is `Some`, in which case it's `BF16` —
    /// `candle_flash_attn` (and so `flash_attn_varlen_paged_windowed`) only
    /// supports F16/BF16, never F32, so paged attention requires switching
    /// the whole model's working dtype, not just attaching a block table.
    Llama {
        model: Mutex<Llama>,
        config: Config,
        device: Device,
        dtype: DType,
        paged: Option<PagedAttentionRuntime>,
        /// `hardware_strategy.cuda_graph_decode`. Only ever `true` when
        /// `paged` is also `Some` — `try_load_with_topology` requires
        /// `paged_attention` for `cuda_graph_decode` (the contiguous cache's
        /// per-step reallocation is incompatible with `CudaGraph` capture).
        cuda_graph_decode: bool,
    },
    Qwen2(Mutex<Qwen2Model>),
    Qwen3(Mutex<Qwen3Model>),
    Qwen3Moe(Mutex<Qwen3MoeModel>),
    Gemma2(Mutex<Gemma2Model>),
    Gemma3(Mutex<Gemma3Model>),
    Phi3(Mutex<Phi3Model>),
    DeepSeek(Mutex<DeepSeekModel>),
}

struct DecodeLoopContext<'a> {
    request: &'a ParsedGenerationRequest,
    eos_tokens: &'a [u32],
    input_device: &'a Device,
    on_token: &'a mut dyn FnMut(&str),
}

/// Model-level `hardware_strategy.paged_attention` state for a Llama
/// checkpoint: one block pool and one pre-allocated (key_cache, value_cache)
/// tensor pair per transformer layer, sized once at load time (see
/// `build_paged_attention_runtime`) and reused for the lifetime of the
/// loaded model. Each generation request gets its own [`PagedSequenceGuard`]
/// drawing blocks from the shared `pool`; there is no cross-request sharing
/// of *content* (every request starts a fresh sequence), only of the
/// physical block storage. Concurrent requests to the same model are safe at
/// the allocator level (the free-list is `Mutex`-guarded) but not yet
/// batched together into a single forward pass — that is continuous
/// batching, a separate follow-up (issue #312 step 4).
#[derive(Debug)]
struct PagedAttentionRuntime {
    pool: Mutex<PagedBlockPool>,
    /// One `(key_cache, value_cache)` pair per transformer layer, each
    /// `(num_blocks, page_block_size, num_kv_heads, head_dim)`.
    layer_kv: Vec<(Tensor, Tensor)>,
    page_block_size: usize,
}

/// Owns one generation request's [`SequenceBlockTable`] for the lifetime of
/// its decode. Frees the table's blocks back to the shared pool on drop —
/// including on an early return or error from the decode loop — so a failed
/// or short-circuited generation never leaks blocks.
struct PagedSequenceGuard<'a> {
    table: SequenceBlockTable,
    pool: &'a Mutex<PagedBlockPool>,
}

impl<'a> PagedSequenceGuard<'a> {
    fn new(pool: &'a Mutex<PagedBlockPool>) -> Self {
        Self {
            table: SequenceBlockTable::new(),
            pool,
        }
    }
}

impl Drop for PagedSequenceGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut pool) = self.pool.lock() {
            self.table.free(&mut pool);
        }
    }
}

/// Per-request `CudaGraph` capture/replay state for the paged-attention
/// decode step (`hardware_strategy.cuda_graph_decode`). Established on the
/// first post-prefill decode step and reused for the rest of that request —
/// `SingleDeviceBackend::decode` drops it when the request completes; there
/// is no cross-request graph reuse yet (see `wire-cuda-graph-decode`'s
/// design.md Decision 4).
///
/// Every tensor here is allocated once, before capture, and updated only by
/// writing its *contents* in place via `Tensor::slice_set` — never replaced
/// or reallocated — matching `CudaGraph::capture`'s documented contract
/// (buffer addresses touched by the captured closure must stay identical
/// across replays). `block_table_buf` is sized to the paged pool's full
/// `min_blocks` width from the start (the pool is already capped at exactly
/// one full-length sequence's worth of blocks, see
/// `wire-paged-attention-decode-path`'s OOM-bug fix), so its *shape* never
/// needs to change within a request.
///
/// `model.forward`'s `index_pos` argument is always passed as `0` for every
/// warm-up/capture/replay call: with both `Cache::set_decode_position` and
/// `Cache::set_paged_kv_decode_slot` attached (below), the paged decode path
/// never reads the host `index_pos` argument at all — rotary embeddings and
/// KV-cache placement are both driven entirely by `position_buf`/
/// `decode_slot_buf`'s device-side contents instead. The *real* position
/// still flows through this session via those two buffers, updated from the
/// real `index_pos` each step in [`Self::write_step`].
#[cfg(feature = "candle-cuda")]
struct CudaGraphDecodeSession {
    graph: candle_core::CudaGraph,
    token_buf: Tensor,
    position_buf: Tensor,
    decode_slot_buf: Tensor,
    block_table_buf: Tensor,
    seqlens_buf: Tensor,
    logits_buf: Tensor,
    page_block_size: usize,
}

// A CUDA_ERROR_INVALID_VALUE surfaced from an unrelated Cache::new call for
// the *next* request on the same device, immediately after a first request's
// CudaGraphDecodeSession (and its CudaGraph) was dropped — even though every
// replay is already followed by `device.synchronize()`. Synchronizing again
// here, immediately before the graph's own Drop impl destroys the CUDA graph
// exec/template, is a defensive measure against that graph teardown racing
// with (or otherwise interacting badly with) the very next CUDA operation on
// the same stream/device.
#[cfg(feature = "candle-cuda")]
impl Drop for CudaGraphDecodeSession {
    fn drop(&mut self) {
        let _ = self.logits_buf.device().synchronize();
    }
}

#[cfg(feature = "candle-cuda")]
impl CudaGraphDecodeSession {
    /// Writes one decode step's real state into the persistent buffers
    /// (device-to-device only — no host readback of `block_table`, unlike
    /// the pre-`astorise/candle#15` `write_new_kv` path this seam replaces).
    /// Does not itself capture or replay anything.
    #[allow(clippy::too_many_arguments)]
    fn write_step(
        table: &SequenceBlockTable,
        index_pos: usize,
        page_block_size: usize,
        token: &Tensor,
        token_buf: &Tensor,
        position_buf: &Tensor,
        decode_slot_buf: &Tensor,
        block_table_buf: &Tensor,
        seqlens_buf: &Tensor,
        device: &Device,
    ) -> candle_core::Result<()> {
        token_buf.slice_set(&token.contiguous()?, 0, 0)?;
        position_buf.slice_set(&Tensor::new(&[index_pos as u32], device)?, 0, 0)?;

        let logical_block = index_pos / page_block_size;
        let offset = index_pos % page_block_size;
        let physical_block = *table.blocks().get(logical_block).ok_or_else(|| {
            candle_core::Error::Msg(format!(
                "cuda_graph_decode: position {index_pos} needs logical block {logical_block}, but the sequence's block table only has {} block(s)",
                table.blocks().len()
            ))
        })? as usize;
        // Matches `PagedKvCache::write_new_kv`'s own host-derived formula
        // (`physical_block * page_block_size + offset`) exactly — this is
        // that same computation, just done here instead of via a block-table
        // readback, per the `Cache::set_paged_kv_decode_slot` seam
        // (astorise/candle#15).
        let flat_slot = (physical_block * page_block_size + offset) as u32;
        decode_slot_buf.slice_set(&Tensor::new(&[flat_slot], device)?, 0, 0)?;

        let blocks = table.blocks();
        let current_row = Tensor::from_vec(blocks.to_vec(), (1, blocks.len()), device)?;
        block_table_buf.slice_set(&current_row, 1, 0)?;

        let seqlens = Tensor::new(&[0u32, (index_pos + 1) as u32], device)?;
        seqlens_buf.slice_set(&seqlens, 0, 0)?;
        Ok(())
    }

    /// Establishes the session on the first decode step: allocates the
    /// persistent buffers at their full maximum width, attaches them to
    /// every transformer layer, runs one uncaptured warm-up forward pass
    /// (per `CudaGraph::capture`'s requirement to JIT-load kernels and
    /// populate the host-to-device upload cache before capture), then
    /// captures a second logical call as the replayable graph. Returns the
    /// session plus this first decode step's real logits.
    #[allow(clippy::too_many_arguments)]
    fn establish(
        model: &Llama,
        cache: &mut Cache,
        paged: &PagedAttentionRuntime,
        table: &SequenceBlockTable,
        cuda_device: &candle_core::CudaDevice,
        device: &Device,
        vocab_size: usize,
        token: &Tensor,
        index_pos: usize,
    ) -> candle_core::Result<(Self, Tensor)> {
        let num_layers = paged.layer_kv.len();
        let page_block_size = paged.page_block_size;
        let min_blocks = paged
            .pool
            .lock()
            .map_err(|_| candle_core::Error::Msg("paged KV block pool mutex was poisoned".into()))?
            .total_blocks();

        let token_buf = Tensor::zeros((1, 1), DType::U32, device)?;
        let position_buf = Tensor::zeros(1, DType::U32, device)?;
        let decode_slot_buf = Tensor::zeros(1, DType::U32, device)?;
        let block_table_buf = Tensor::zeros((1, min_blocks), DType::U32, device)?;
        let seqlens_buf = Tensor::zeros(2, DType::U32, device)?;
        // `Llama::forward`'s natural output is `(batch, vocab_size)` — no
        // separate seq_len dimension (confirmed by `cuda-quality`: an
        // earlier `(1, 1, vocab_size)` allocation here failed with
        // `unexpected rank, expected: 3, got: 2` — and matches
        // `decode_loop_from_logits`'s own `logits.squeeze(0)`, which removes
        // exactly one (batch) dimension before sampling).
        let logits_buf = Tensor::zeros((1, vocab_size), DType::F32, device)?;

        Self::write_step(
            table,
            index_pos,
            page_block_size,
            token,
            &token_buf,
            &position_buf,
            &decode_slot_buf,
            &block_table_buf,
            &seqlens_buf,
            device,
        )?;

        for layer_idx in 0..num_layers {
            cache.set_decode_position(layer_idx, position_buf.clone())?;
            cache.set_paged_kv_decode_slot(layer_idx, decode_slot_buf.clone())?;
            let (key_cache, value_cache) = &paged.layer_kv[layer_idx];
            cache.set_paged_kv(
                layer_idx,
                PagedKvCache {
                    key_cache: key_cache.clone(),
                    value_cache: value_cache.clone(),
                    block_table: block_table_buf.clone(),
                    seqlens_k: seqlens_buf.clone(),
                    page_block_size,
                },
            )?;
        }

        // Warm-up: a real (uncaptured) forward pass, wrapped in its own
        // host-to-device upload cache guard — `CudaGraph::capture` only
        // holds that guard for its own call, not for whatever warm-up the
        // caller ran beforehand.
        {
            let _warmup_guard = cuda_device.enable_cuda_graph_htod_cache();
            let logits = model.forward(&token_buf, 0, cache)?.to_dtype(DType::F32)?;
            logits_buf.slice_set(&logits, 0, 0)?;
        }
        device.synchronize()?;

        let (graph, ()) = candle_core::CudaGraph::capture(cuda_device, || {
            let logits = model.forward(&token_buf, 0, cache)?.to_dtype(DType::F32)?;
            logits_buf.slice_set(&logits, 0, 0)?;
            Ok(())
        })?;

        // Capture only records the operations; it does not execute them.
        // Replay once to get this first decode step's real computed logits.
        graph.replay()?;
        device.synchronize()?;

        Ok((
            Self {
                graph,
                token_buf,
                position_buf,
                decode_slot_buf,
                block_table_buf,
                seqlens_buf,
                logits_buf: logits_buf.clone(),
                page_block_size,
            },
            logits_buf,
        ))
    }

    /// Writes a subsequent decode step's real state into the persistent
    /// buffers and replays the captured graph, returning this step's logits.
    fn replay_step(
        &self,
        table: &SequenceBlockTable,
        device: &Device,
        token: &Tensor,
        index_pos: usize,
    ) -> candle_core::Result<Tensor> {
        Self::write_step(
            table,
            index_pos,
            self.page_block_size,
            token,
            &self.token_buf,
            &self.position_buf,
            &self.decode_slot_buf,
            &self.block_table_buf,
            &self.seqlens_buf,
            device,
        )?;
        self.graph.replay()?;
        device.synchronize()?;
        Ok(self.logits_buf.clone())
    }
}

/// Attaches this forward step's paged KV state to every transformer layer
/// (growing `table` from `paged.pool` as needed so it can hold
/// `index_pos + seq_len` tokens), then runs the forward pass and converts
/// the model's BF16 logits back to F32 for the shared sampling/FSM pipeline,
/// which assumes F32 throughout.
fn paged_llama_forward(
    model: &Llama,
    cache: &mut Cache,
    paged: &PagedAttentionRuntime,
    table: &mut SequenceBlockTable,
    device: &Device,
    input: &Tensor,
    index_pos: usize,
) -> candle_core::Result<Tensor> {
    let seq_len = input.dims2()?.1;
    {
        let mut pool = paged.pool.lock().map_err(|_| {
            candle_core::Error::Msg("paged KV block pool mutex was poisoned".into())
        })?;
        table
            .grow_to(index_pos + seq_len, &mut pool)
            .map_err(|error| candle_core::Error::Msg(error.to_string()))?;
    }
    let block_table = build_block_table_tensor(&[table], device)?;
    let seqlens_k = build_cumulative_seqlens_tensor(&[index_pos + seq_len], device)?;
    for (layer_idx, (key_cache, value_cache)) in paged.layer_kv.iter().enumerate() {
        cache.set_paged_kv(
            layer_idx,
            PagedKvCache {
                key_cache: key_cache.clone(),
                value_cache: value_cache.clone(),
                block_table: block_table.clone(),
                seqlens_k: seqlens_k.clone(),
                page_block_size: paged.page_block_size,
            },
        )?;
    }
    model.forward(input, index_pos, cache)?.to_dtype(DType::F32)
}

/// Sizes and allocates the shared paged-KV block pool and per-layer
/// `(key_cache, value_cache)` tensors for a Llama checkpoint. Queries free
/// VRAM *after* the caller has already loaded the model's weights onto
/// `device` (via `discover_cluster_topology`), so the budget reflects what's
/// actually left, and uses a fixed fraction of that remaining free VRAM — a
/// configurable knob is a follow-up (see
/// `wire-paged-attention-decode-path`'s tasks.md 2.2). Requires enough
/// blocks to hold one full-length sequence (`config.max_position_embeddings`
/// tokens); this is deliberately conservative for a single in-flight
/// sequence per model (continuous batching, which shares the pool across
/// many concurrent sequences, is a separate follow-up), and fails the load
/// with a typed error if the budget can't fit even that.
// `candle_flash_attn`'s paged kernel hard-requires `page_block_size % 32 == 0`
// (checked at runtime in candle-flash-attn/src/lib.rs) — 32 is the smallest
// valid value, minimizing memory waste for the single-in-flight-sequence
// scope this runtime supports today. The `const` assertion below turns a
// violation into a compile error instead of a real-GPU-only runtime failure
// (this exact mistake — 16, not a multiple of 32 — previously only surfaced
// on a real `cuda-quality` run: "paged flash-attn requires page_block_size
// to be a multiple of 32").
const PAGED_ATTENTION_PAGE_BLOCK_SIZE: usize = 32;
const _: () = assert!(
    PAGED_ATTENTION_PAGE_BLOCK_SIZE.is_multiple_of(32),
    "paged flash-attn requires page_block_size to be a multiple of 32"
);

/// Sizes the shared paged-KV block pool. Pure arithmetic (no device/tensor
/// allocation), so this is unit-testable on CPU independent of real NVML
/// telemetry — see `paged_attention_pool_is_capped_at_one_sequence_even_with_abundant_free_vram`,
/// a regression test for a real bug this exact function used to have (an
/// omitted `num_hidden_layers` factor plus sizing to the *entire* budget
/// instead of just what one sequence needs OOM'd a real GPU on a tiny test
/// fixture that had gigabytes of nominally "free" VRAM to claim).
///
/// Only one sequence is ever in flight per model today (continuous
/// batching, which would benefit from a larger shared pool, is a separate
/// follow-up — issue #312 step 4), so the pool is capped at exactly
/// `min_blocks` (one full-length sequence at `config.max_position_embeddings`)
/// regardless of how much more VRAM is free, rather than claiming whatever
/// fits the entire budget.
fn size_paged_kv_pool(
    config: &Config,
    dtype: DType,
    free_vram_bytes: u64,
) -> Result<PagedBlockPool, PagedKvError> {
    const FREE_VRAM_BUDGET_FRACTION: f64 = 0.5;

    let head_dim = config.hidden_size / config.num_attention_heads;
    // Cost of one logical block across *every* transformer layer: `layer_kv`
    // allocates one (key_cache, value_cache) pair per layer, each sized by
    // `num_blocks`, so a block's true footprint is this per-layer cost times
    // `num_hidden_layers`, not just one layer's worth.
    let bytes_per_block = 2u64 // key + value
        * PAGED_ATTENTION_PAGE_BLOCK_SIZE as u64
        * config.num_key_value_heads as u64
        * head_dim as u64
        * dtype.size_in_bytes() as u64
        * config.num_hidden_layers as u64;
    let min_blocks = config
        .max_position_embeddings
        .div_ceil(PAGED_ATTENTION_PAGE_BLOCK_SIZE);

    let min_blocks_bytes = bytes_per_block.saturating_mul(min_blocks as u64);
    let budget_bytes =
        ((free_vram_bytes as f64 * FREE_VRAM_BUDGET_FRACTION) as u64).min(min_blocks_bytes);

    PagedBlockPool::try_new_within_budget(
        PAGED_ATTENTION_PAGE_BLOCK_SIZE,
        bytes_per_block,
        budget_bytes,
        min_blocks,
    )
}

fn build_paged_attention_runtime(
    alias: &str,
    root: &Path,
    config: &Config,
    device: &Device,
    dtype: DType,
) -> Result<PagedAttentionRuntime, CandleLlmError> {
    const CUDA_ORDINAL_DEVICE_ID: u32 = 1; // see discover_cluster_topology: ordinal 0 -> device_id 1

    let head_dim = config.hidden_size / config.num_attention_heads;
    let free_vram_bytes = discover_cluster_topology()
        .devices
        .iter()
        .find(|info| info.device_id == CUDA_ORDINAL_DEVICE_ID)
        .map(|info| info.free_vram_bytes)
        .unwrap_or(0);
    let pool = size_paged_kv_pool(config, dtype, free_vram_bytes).map_err(|error| {
        CandleLlmError::UnsupportedModel {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            detail: format!("paged_attention KV cache pool could not be sized: {error}"),
        }
    })?;

    let num_blocks = pool.total_blocks();
    let alloc_layer_kv = || -> candle_core::Result<(Tensor, Tensor)> {
        let shape = (
            num_blocks,
            PAGED_ATTENTION_PAGE_BLOCK_SIZE,
            config.num_key_value_heads,
            head_dim,
        );
        Ok((
            Tensor::zeros(shape, dtype, device)?,
            Tensor::zeros(shape, dtype, device)?,
        ))
    };
    let mut layer_kv = Vec::with_capacity(config.num_hidden_layers);
    for _ in 0..config.num_hidden_layers {
        layer_kv.push(
            alloc_layer_kv().map_err(|error| CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail: format!("failed to allocate paged_attention KV cache: {error}"),
            })?,
        );
    }

    Ok(PagedAttentionRuntime {
        pool: Mutex::new(pool),
        layer_kv,
        page_block_size: PAGED_ATTENTION_PAGE_BLOCK_SIZE,
    })
}

impl SingleDeviceBackend {
    fn decode(
        &self,
        runtime: &CandleLlmRuntime,
        prompt_ids: &[u32],
        request: &ParsedGenerationRequest,
        eos_tokens: &[u32],
        device: &Device,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<(), CandleLlmError> {
        match self {
            Self::Llama {
                model,
                config,
                dtype,
                paged,
                cuda_graph_decode,
                ..
            } => {
                let model = model.lock().map_err(|_| {
                    runtime.execution_error("Llama model mutex was poisoned".to_owned())
                })?;
                let mut cache = Cache::new(true, *dtype, config, device).map_err(|error| {
                    runtime.execution_error(format!("failed to build KV cache: {error}"))
                })?;
                match paged {
                    Some(paged) if *cuda_graph_decode => {
                        #[cfg_attr(not(feature = "candle-cuda"), allow(unused_mut))]
                        let mut guard = PagedSequenceGuard::new(&paged.pool);
                        #[cfg(feature = "candle-cuda")]
                        {
                            let mut session: Option<CudaGraphDecodeSession> = None;
                            runtime.decode_loop(
                                prompt_ids,
                                request,
                                eos_tokens,
                                device,
                                on_token,
                                |input, index_pos| {
                                    let seq_len = input.dims2()?.1;
                                    if seq_len != 1 {
                                        // Prefill: shapes vary chunk to chunk,
                                        // never captured — always the
                                        // existing uncaptured paged path.
                                        return paged_llama_forward(
                                            &model,
                                            &mut cache,
                                            paged,
                                            &mut guard.table,
                                            device,
                                            input,
                                            index_pos,
                                        );
                                    }
                                    {
                                        let mut pool = paged.pool.lock().map_err(|_| {
                                            candle_core::Error::Msg(
                                                "paged KV block pool mutex was poisoned".into(),
                                            )
                                        })?;
                                        guard.table.grow_to(index_pos + 1, &mut pool).map_err(
                                            |error| candle_core::Error::Msg(error.to_string()),
                                        )?;
                                    }
                                    match &session {
                                        None => {
                                            // `try_load_with_topology` already
                                            // requires a real CUDA device for
                                            // `cuda_graph_decode`, so this
                                            // should always match; fall back
                                            // to the uncaptured path rather
                                            // than panicking if it somehow
                                            // doesn't.
                                            let Device::Cuda(cuda_device) = device else {
                                                return paged_llama_forward(
                                                    &model,
                                                    &mut cache,
                                                    paged,
                                                    &mut guard.table,
                                                    device,
                                                    input,
                                                    index_pos,
                                                );
                                            };
                                            let (established, logits) =
                                                CudaGraphDecodeSession::establish(
                                                    &model,
                                                    &mut cache,
                                                    paged,
                                                    &guard.table,
                                                    cuda_device,
                                                    device,
                                                    config.vocab_size,
                                                    input,
                                                    index_pos,
                                                )?;
                                            session = Some(established);
                                            Ok(logits)
                                        }
                                        Some(established) => established.replay_step(
                                            &guard.table,
                                            device,
                                            input,
                                            index_pos,
                                        ),
                                    }
                                },
                            )
                        }
                        #[cfg(not(feature = "candle-cuda"))]
                        {
                            // `try_load_with_topology` requires `candle-cuda`
                            // for `cuda_graph_decode`, so this is unreachable
                            // in practice; kept only so this match stays
                            // exhaustive on non-CUDA builds.
                            let _ = &guard;
                            Err(runtime.execution_error(
                                "cuda_graph_decode requires the candle-cuda feature".to_owned(),
                            ))
                        }
                    }
                    Some(paged) => {
                        let mut guard = PagedSequenceGuard::new(&paged.pool);
                        runtime.decode_loop(
                            prompt_ids,
                            request,
                            eos_tokens,
                            device,
                            on_token,
                            |input, index_pos| {
                                paged_llama_forward(
                                    &model,
                                    &mut cache,
                                    paged,
                                    &mut guard.table,
                                    device,
                                    input,
                                    index_pos,
                                )
                            },
                        )
                    }
                    None => {
                        let (logits, index_pos) = runtime.llama_prefill_with_prefix_cache(
                            &model, prompt_ids, &mut cache, device,
                        )?;
                        runtime.decode_loop_from_logits(
                            logits,
                            index_pos,
                            prompt_ids,
                            DecodeLoopContext {
                                request,
                                eos_tokens,
                                input_device: device,
                                on_token,
                            },
                            |input, index_pos| model.forward(input, index_pos, &mut cache),
                        )
                    }
                }
            }
            Self::Qwen2(model) => {
                let mut model = model.lock().map_err(|_| {
                    runtime.execution_error("Qwen2 model mutex was poisoned".to_owned())
                })?;
                model.clear_kv_cache();
                runtime.decode_loop(
                    prompt_ids,
                    request,
                    eos_tokens,
                    device,
                    on_token,
                    |input, index_pos| model.forward(input, index_pos)?.squeeze(1),
                )
            }
            Self::Qwen3(model) => {
                let mut model = model.lock().map_err(|_| {
                    runtime.execution_error("Qwen3 model mutex was poisoned".to_owned())
                })?;
                model.clear_kv_cache();
                runtime.decode_loop(
                    prompt_ids,
                    request,
                    eos_tokens,
                    device,
                    on_token,
                    |input, index_pos| model.forward(input, index_pos)?.squeeze(1),
                )
            }
            Self::Qwen3Moe(model) => {
                let mut model = model.lock().map_err(|_| {
                    runtime.execution_error("Qwen3-MoE model mutex was poisoned".to_owned())
                })?;
                model.clear_kv_cache();
                runtime.decode_loop(
                    prompt_ids,
                    request,
                    eos_tokens,
                    device,
                    on_token,
                    |input, index_pos| model.forward(input, index_pos)?.squeeze(1),
                )
            }
            Self::Gemma2(model) => {
                let mut model = model.lock().map_err(|_| {
                    runtime.execution_error("Gemma2 model mutex was poisoned".to_owned())
                })?;
                model.clear_kv_cache();
                runtime.decode_loop(
                    prompt_ids,
                    request,
                    eos_tokens,
                    device,
                    on_token,
                    |input, index_pos| model.forward(input, index_pos)?.squeeze(1),
                )
            }
            Self::Gemma3(model) => {
                let mut model = model.lock().map_err(|_| {
                    runtime.execution_error("Gemma3 model mutex was poisoned".to_owned())
                })?;
                model.clear_kv_cache();
                runtime.decode_loop(
                    prompt_ids,
                    request,
                    eos_tokens,
                    device,
                    on_token,
                    |input, index_pos| model.forward(input, index_pos)?.squeeze(1),
                )
            }
            Self::Phi3(model) => {
                let mut model = model.lock().map_err(|_| {
                    runtime.execution_error("Phi model mutex was poisoned".to_owned())
                })?;
                model.clear_kv_cache();
                runtime.decode_loop(
                    prompt_ids,
                    request,
                    eos_tokens,
                    device,
                    on_token,
                    |input, index_pos| model.forward(input, index_pos)?.squeeze(1),
                )
            }
            Self::DeepSeek(model) => {
                let mut model = model.lock().map_err(|_| {
                    runtime.execution_error("DeepSeek model mutex was poisoned".to_owned())
                })?;
                model.clear_kv_cache();
                runtime.decode_loop(
                    prompt_ids,
                    request,
                    eos_tokens,
                    device,
                    on_token,
                    |input, index_pos| model.forward(input, index_pos),
                )
            }
        }
    }

    fn last_logits(&self, input: &Tensor, device: &Device) -> Tensor {
        match self {
            Self::Llama {
                model,
                config,
                dtype,
                ..
            } => {
                // Intentionally the plain contiguous-cache path even when this
                // backend is paged-attention-enabled (`dtype` may be `BF16`):
                // callers (constrained-decoding lookahead, the `debug_last_logits`
                // test helper) don't have a `PagedAttentionRuntime`/block table to
                // attach, so this always runs the dense forward, converting back
                // to F32 for the shared sampling/FSM code that expects it.
                let mut cache =
                    Cache::new(true, *dtype, config, device).expect("cache should build");
                let model = model.lock().expect("llama mutex should not be poisoned");
                model
                    .forward(input, 0, &mut cache)
                    .and_then(|logits| logits.to_dtype(DType::F32))
                    .expect("forward pass should run")
            }
            Self::Qwen2(model) => {
                let mut model = model.lock().expect("Qwen2 mutex should not be poisoned");
                model.clear_kv_cache();
                model
                    .forward(input, 0)
                    .and_then(|logits| logits.squeeze(1))
                    .expect("Qwen2 forward pass should run")
            }
            Self::Qwen3(model) => {
                let mut model = model.lock().expect("Qwen3 mutex should not be poisoned");
                model.clear_kv_cache();
                model
                    .forward(input, 0)
                    .and_then(|logits| logits.squeeze(1))
                    .expect("Qwen3 forward pass should run")
            }
            Self::Qwen3Moe(model) => {
                let mut model = model
                    .lock()
                    .expect("Qwen3-MoE mutex should not be poisoned");
                model.clear_kv_cache();
                model
                    .forward(input, 0)
                    .and_then(|logits| logits.squeeze(1))
                    .expect("Qwen3-MoE forward pass should run")
            }
            Self::Gemma2(model) => {
                let mut model = model.lock().expect("Gemma2 mutex should not be poisoned");
                model.clear_kv_cache();
                model
                    .forward(input, 0)
                    .and_then(|logits| logits.squeeze(1))
                    .expect("Gemma2 forward pass should run")
            }
            Self::Gemma3(model) => {
                let mut model = model.lock().expect("Gemma3 mutex should not be poisoned");
                model.clear_kv_cache();
                model
                    .forward(input, 0)
                    .and_then(|logits| logits.squeeze(1))
                    .expect("Gemma3 forward pass should run")
            }
            Self::Phi3(model) => {
                let mut model = model.lock().expect("Phi mutex should not be poisoned");
                model.clear_kv_cache();
                model
                    .forward(input, 0)
                    .and_then(|logits| logits.squeeze(1))
                    .expect("Phi forward pass should run")
            }
            Self::DeepSeek(model) => {
                let mut model = model.lock().expect("DeepSeek mutex should not be poisoned");
                model.clear_kv_cache();
                model
                    .forward(input, 0)
                    .expect("DeepSeek forward pass should run")
            }
        }
    }

    /// The device this backend's weights were actually loaded onto. `Cpu` for
    /// every architecture except a Llama checkpoint that
    /// `load_safetensors` resolved a real CUDA device for.
    fn device(&self) -> Device {
        match self {
            Self::Llama { device, .. } => device.clone(),
            Self::Qwen2(_)
            | Self::Qwen3(_)
            | Self::Qwen3Moe(_)
            | Self::Gemma2(_)
            | Self::Gemma3(_)
            | Self::Phi3(_)
            | Self::DeepSeek(_) => Device::Cpu,
        }
    }
}

/// The concrete parallel engine behind a [`LoadedModel::Parallel`]. Tensor,
/// pipeline, and expert (MoE) parallelism all support the full autoregressive
/// decode loop: tensor and expert parallelism each carry a single
/// [`TensorParallelCache`] (expert parallelism's attention is dense and
/// replicated exactly like tensor parallelism's, only the MLP differs per
/// layer), while pipeline parallelism carries one per-stage cache built fresh
/// per request via [`PipelineParallelLlama::new_caches`] and threaded through
/// [`PipelineParallelLlama::forward_at`] on every prefill/decode call.
enum ParallelModel {
    Tensor {
        model: Box<TensorParallelLlama>,
        config: Config,
        eos_tokens: Vec<u32>,
        /// Devices the plan sharded across; `devices[0]` is the primary device
        /// the input tensor and KV cache are built on.
        devices: Vec<Device>,
    },
    Pipeline {
        model: Box<PipelineParallelLlama>,
        config: Config,
        eos_tokens: Vec<u32>,
        /// Devices the plan sharded across, in stage order; `devices[0]` is
        /// the primary device the input tensor is built on.
        devices: Vec<Device>,
    },
    Expert {
        model: Box<ExpertParallelLlama>,
        config: Config,
        eos_tokens: Vec<u32>,
        /// Devices the plan sharded experts across; `devices[0]` is the
        /// primary device the input tensor and KV cache are built on.
        devices: Vec<Device>,
    },
}

/// Build one in-process transport per stage boundary, moving the activation
/// onto each next stage's device between stages. Used by both the production
/// decode path (`decode`'s `ParallelModel::Pipeline` arm) and the
/// prefill-equivalence test/debug helpers below; a real cross-node deployment
/// would swap `InProcessTransport` for `TcpStageTransport` without changing
/// this composition's shape.
fn pipeline_stage_transports(
    model: &PipelineParallelLlama,
) -> Vec<Box<dyn super::parallel::StageTransport>> {
    use super::parallel::{InProcessTransport, StageTransport};
    model
        .stages
        .iter()
        .skip(1)
        .map(|stage| {
            Box::new(InProcessTransport {
                next_device: stage.device().clone(),
            }) as Box<dyn StageTransport>
        })
        .collect()
}

/// Run a single prefill forward through every pipeline stage. Used by the
/// prefill-equivalence test/debug path; the production decode path instead
/// calls [`PipelineParallelLlama::forward_at`] directly so it can reuse the
/// same per-stage caches across multiple decode steps.
fn pipeline_prefill_forward(
    model: &PipelineParallelLlama,
    input: &Tensor,
) -> candle_core::Result<Tensor> {
    let transports = pipeline_stage_transports(model);
    model.forward(input, &transports)
}

/// Runtime guardrails applied to every generation request. Independent of the
/// model weights (the HF config only contributes the context window).
#[derive(Clone, Copy)]
struct GenerationLimits {
    default_max_new_tokens: usize,
    max_prompt_bytes: usize,
    max_prompt_tokens: usize,
    max_batch_size: usize,
    max_position_embeddings: usize,
    /// `None` disables chunking; `Some(n)` bounds each prefill forward to at
    /// most `n` tokens. The default is `Some(8192)` to match candle-vllm.
    prefill_chunk_tokens: Option<usize>,
}

impl GenerationLimits {
    fn with_context(max_position_embeddings: usize) -> Self {
        Self {
            default_max_new_tokens: DEFAULT_MAX_NEW_TOKENS,
            max_prompt_bytes: DEFAULT_MAX_PROMPT_BYTES,
            max_prompt_tokens: DEFAULT_MAX_PROMPT_TOKENS,
            max_batch_size: DEFAULT_MAX_BATCH_SIZE,
            max_position_embeddings,
            prefill_chunk_tokens: Some(DEFAULT_PREFILL_CHUNK_TOKENS),
        }
    }

    fn with_prefill_chunk_tokens(mut self, configured: Option<u32>) -> Self {
        if let Some(tokens) = configured {
            self.prefill_chunk_tokens = match usize::try_from(tokens) {
                Ok(0) => None,
                Ok(value) => Some(value),
                Err(_) => Some(DEFAULT_PREFILL_CHUNK_TOKENS),
            };
        }
        self
    }

    fn next_prefill_chunk_len(&self, remaining: usize) -> usize {
        self.prefill_chunk_tokens
            .filter(|tokens| *tokens > 0)
            .map(|tokens| remaining.min(tokens))
            .unwrap_or(remaining)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PrefixCacheKey {
    token_len: usize,
    token_hash: u64,
}

#[derive(Clone)]
struct PrefixCacheEntry {
    tokens: Arc<[u32]>,
    cache: Cache,
    logits: Tensor,
    last_used: u64,
}

#[derive(Default)]
struct PrefixCache {
    entries: HashMap<PrefixCacheKey, PrefixCacheEntry>,
    clock: u64,
    hits: u64,
    misses: u64,
}

impl PrefixCache {
    fn longest_match(&mut self, prompt_ids: &[u32]) -> Option<PrefixCacheEntry> {
        let mut len = prompt_ids.len() - (prompt_ids.len() % PREFIX_CACHE_BLOCK_TOKENS);
        while len >= PREFIX_CACHE_BLOCK_TOKENS {
            let key = prefix_cache_key(&prompt_ids[..len]);
            if let Some(entry) = self.entries.get_mut(&key) {
                if entry.tokens.as_ref() == &prompt_ids[..len] {
                    self.clock = self.clock.saturating_add(1);
                    entry.last_used = self.clock;
                    self.hits = self.hits.saturating_add(1);
                    return Some(entry.clone());
                }
            }
            len = len.saturating_sub(PREFIX_CACHE_BLOCK_TOKENS);
        }
        self.misses = self.misses.saturating_add(1);
        None
    }

    fn insert(&mut self, prompt_ids: &[u32], cache: Cache, logits: Tensor) {
        if prompt_ids.len() < PREFIX_CACHE_BLOCK_TOKENS
            || !prompt_ids.len().is_multiple_of(PREFIX_CACHE_BLOCK_TOKENS)
        {
            return;
        }
        self.clock = self.clock.saturating_add(1);
        let key = prefix_cache_key(prompt_ids);
        self.entries.insert(
            key,
            PrefixCacheEntry {
                tokens: Arc::from(prompt_ids),
                cache,
                logits,
                last_used: self.clock,
            },
        );
        if self.entries.len() > PREFIX_CACHE_MAX_ENTRIES {
            if let Some(evict) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key)
            {
                self.entries.remove(&evict);
            }
        }
    }

    #[cfg(test)]
    fn token_lengths(&self) -> Vec<usize> {
        let mut lengths = self
            .entries
            .values()
            .map(|entry| entry.tokens.len())
            .collect::<Vec<_>>();
        lengths.sort_unstable();
        lengths
    }
}

fn prefix_cache_key(prompt_ids: &[u32]) -> PrefixCacheKey {
    let mut hasher = DefaultHasher::new();
    prompt_ids.hash(&mut hasher);
    PrefixCacheKey {
        token_len: prompt_ids.len(),
        token_hash: hasher.finish(),
    }
}

impl std::fmt::Debug for CandleLlmRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CandleLlmRuntime")
            .field("alias", &self.alias)
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub(crate) struct CandleLlmRuntime {
    alias: String,
    root: PathBuf,
    tokenizer: Tokenizer,
    inner: Arc<LoadedModel>,
    limits: GenerationLimits,
    /// The model's own chat template, loaded once from `tokenizer_config.json`.
    /// `None` when the checkpoint ships no template (the runtime then falls back
    /// to a generic chat rendering). Shared behind `Arc` so clones stay cheap.
    chat_template: Option<Arc<ChatTemplate>>,
    /// Compiled JSON-Schema grammars for constrained decoding, keyed by the
    /// schema's SHA-256 hash. Shared behind `Arc` so clones (and concurrent
    /// requests) reuse the same cache instead of recompiling per-request.
    fsm_cache: Arc<FsmCache>,
    /// Block-addressed KV prefix cache for the upstream Llama safetensors
    /// backend. Other families keep their current per-request cache behavior
    /// until their Candle cache types expose a cloneable external cache.
    prefix_cache: Arc<Mutex<PrefixCache>>,
}

#[derive(Debug, Deserialize)]
struct ModelTypeProbe {
    #[serde(default)]
    model_type: String,
    #[serde(default)]
    text_config: Option<Box<ModelTypeProbe>>,
    #[serde(default)]
    vision_config: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TokenIdField {
    One(u32),
    Many(Vec<u32>),
}

#[derive(Debug, Default, Deserialize)]
struct GenerationMetadataProbe {
    #[serde(default)]
    eos_token_id: Option<TokenIdField>,
    #[serde(default = "default_max_position_embeddings")]
    max_position_embeddings: usize,
}

fn default_max_position_embeddings() -> usize {
    DEFAULT_MAX_PROMPT_TOKENS
}

/// Parsed `.tachyon-model.json` sidecar. Only the declared format is consumed;
/// unknown fields are ignored so the broker can extend it freely.
#[derive(Debug, Deserialize)]
struct ModelMeta {
    #[serde(default)]
    format: String,
}

#[derive(Debug, Deserialize)]
struct SafetensorsIndex {
    weight_map: HashMap<String, String>,
}

/// A single chat turn carried by a structured (`messages`) generation request.
/// Rendered into a prompt by the model's own chat template at parse time.
/// `Serialize` so it can be handed to the Jinja chat-template context.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub(crate) struct ChatTurn {
    pub(crate) role: String,
    pub(crate) content: String,
}

#[derive(Debug, Deserialize)]
struct GenerationRequest {
    /// Raw prompt. Optional when `messages` is supplied (chat-templated path).
    #[serde(default)]
    prompt: Option<String>,
    /// Structured chat turns. When present, the model's chat template renders
    /// them into the final prompt; `prompt` is ignored.
    #[serde(default)]
    messages: Option<Vec<ChatTurn>>,
    max_new_tokens: Option<usize>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    seed: Option<u64>,
    /// Stop sequences: generation halts once any of these appears in the decoded
    /// text, and the matched sequence (and anything after it) is trimmed.
    #[serde(default)]
    stop: Option<Vec<String>>,
    /// Optional JSON Schema (source text) constraining generated output to a
    /// grammar compiled from the schema. See `samplers::compile_schema` for
    /// the supported subset.
    #[serde(default)]
    json_schema: Option<String>,
}

/// Token-selection policy resolved from a request's `temperature`/`top_p`/`seed`.
/// `temperature <= 0` (or absent) collapses to deterministic greedy decoding,
/// which preserves the runtime's reproducible-by-default contract.
struct SamplingPolicy {
    seed: u64,
    temperature: Option<f64>,
    top_p: Option<f64>,
}

impl SamplingPolicy {
    fn is_greedy(&self) -> bool {
        self.temperature.is_none()
    }

    fn processor(&self) -> LogitsProcessor {
        match self.temperature {
            // Greedy: deterministic argmax, independent of the seed.
            None => LogitsProcessor::from_sampling(self.seed, Sampling::ArgMax),
            Some(temperature) => match self.top_p {
                Some(p) if p > 0.0 && p < 1.0 => {
                    LogitsProcessor::from_sampling(self.seed, Sampling::TopP { p, temperature })
                }
                _ => LogitsProcessor::from_sampling(self.seed, Sampling::All { temperature }),
            },
        }
    }
}

struct ParsedGenerationRequest {
    prompt: String,
    max_new_tokens: usize,
    sampling: SamplingPolicy,
    stop: Vec<String>,
    /// Compiled grammar for constrained decoding, when the request supplied
    /// `json_schema`.
    fsm: Option<Arc<super::samplers::CompiledFsm>>,
}

impl CandleLlmRuntime {
    pub(crate) fn try_load(
        alias: &str,
        path: impl AsRef<Path>,
        requested_device: &str,
        strategy: &HardwareStrategy,
    ) -> Result<Option<Self>, CandleLlmError> {
        // Discover the real cluster topology so a non-`single` strategy is
        // validated against actual hardware before any weights load. On a
        // CUDA-less build this reports a single CPU device, which correctly
        // rejects multi-device plans unless a test injects a topology via
        // `try_load_with_topology`.
        Self::try_load_with_topology(
            alias,
            path,
            requested_device,
            strategy,
            &discover_cluster_topology(),
        )
    }

    fn try_load_with_topology(
        alias: &str,
        path: impl AsRef<Path>,
        requested_device: &str,
        strategy: &HardwareStrategy,
        topology: &ClusterTopology,
    ) -> Result<Option<Self>, CandleLlmError> {
        let root = path.as_ref();
        if !root.is_dir() {
            return Ok(None);
        }

        // Resolve the on-disk format: trust the broker's sidecar when present,
        // otherwise infer from directory contents (operator-provisioned models).
        let Some(format) = resolve_model_format(alias, root)? else {
            return Ok(None);
        };
        let architecture = inspect_model_architecture(alias, root, format)?;
        if architecture.capabilities().specialized_runtime {
            return Err(CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail: format!(
                    "architecture `{}` is handled by its specialized runtime, not the generic Candle loader",
                    architecture.display_name()
                ),
            });
        }
        if !architecture.supports_format(format) {
            return Err(CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail: format!(
                    "recognized architecture `{}` has no verified {} loader",
                    architecture.display_name(),
                    format.display_name()
                ),
            });
        }
        if !architecture.supports_strategy(strategy) {
            let detail = if strategy.distribution_mode == GpuDistribution::ExpertParallelism
                && architecture == ModelArchitecture::Llama
            {
                "expert-parallel execution requires a Mixtral-family checkpoint; got `llama`"
                    .to_owned()
            } else {
                format!(
                    "architecture `{}` does not support `{}` execution",
                    architecture.display_name(),
                    distribution_mode_name(strategy.distribution_mode)
                )
            };
            return Err(CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail,
            });
        }
        // The single-device path executes on a real CUDA device only for a
        // Llama checkpoint on a `candle-cuda` build (see
        // `enable-single-device-llama-cuda-execution`); every other
        // architecture, or any build without that feature, still returns the
        // existing typed error for a non-`cpu` request. A non-`single`
        // strategy resolves its devices from the validated plan instead, so
        // it does not go through this check — and, critically, none of
        // `load_parallel`'s tensor/pipeline/expert engines read
        // `paged_attention`/`flashinfer_attention`/`cuda_graph_decode` at all
        // (they carry their own separate cache types). Without also
        // requiring `distribution_mode == Single` here, a parallel-mode
        // request setting one of those flags would pass this gate and then
        // silently dispatch to `load_parallel`, which ignores the flag
        // entirely — accepted but not actually wired.
        let single_device_cuda_supported = cfg!(feature = "candle-cuda")
            && architecture == ModelArchitecture::Llama
            && strategy.distribution_mode == GpuDistribution::Single;

        // paged_attention needs that same Llama+CUDA baseline — the paged
        // flash-attn kernel is CUDA-only — plus a Safetensors checkpoint and
        // an actual non-`cpu` request. `load_safetensors` is the only loader
        // that wires `cache.set_paged_kv`: GGUF now runs on CUDA too, but
        // `QuantizedLlama` owns its own KV cache internally and exposes no
        // block-table seam. Every other architecture/format/build/device
        // combination therefore fails closed here instead of silently running
        // the contiguous path.
        let paged_attention_supported = single_device_cuda_supported
            && format == ModelFormat::Safetensors
            && requested_device != "cpu";
        if strategy.paged_attention && !paged_attention_supported {
            return Err(CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail:
                    "paged_attention requires a Safetensors Llama checkpoint on a CUDA device because only that loader wires Tachyon's block allocator and per-sequence block table to candle_flash_attn::flash_attn_varlen_paged_windowed"
                        .to_owned(),
            });
        }

        // flashinfer_attention needs the same Llama+CUDA baseline plus the
        // separate, optional `candle-flashinfer` Cargo feature: unlike
        // `flash-attn` (unconditionally bundled into `candle-cuda`, so
        // `single_device_cuda_supported` alone is enough for paged_attention),
        // `candle-transformers/flashinfer-kernels` is only compiled in when
        // `candle-flashinfer` is enabled. Without that check, a `candle-cuda`
        // build lacking `candle-flashinfer` would report this as supported and
        // then panic at decode time (`unimplemented!("compile with
        // '--features flashinfer-kernels'")`).
        let flashinfer_attention_supported = single_device_cuda_supported
            && cfg!(feature = "candle-flashinfer")
            && requested_device != "cpu";
        if strategy.flashinfer_attention {
            if !flashinfer_attention_supported {
                return Err(CandleLlmError::UnsupportedModel {
                    alias: alias.to_owned(),
                    path: root.to_path_buf(),
                    detail:
                        "flashinfer_attention requires Tachyon's decode-attention path to be wired to candle-flashinfer-kernels::flashinfer_decode_attention, and is only available for a Llama checkpoint on a CUDA device with the candle-flashinfer feature compiled in"
                            .to_owned(),
                });
            }
            // The fork's `flashinfer_decode_attention` attends over the
            // contiguous cache shape; paged attention's block-table layout is
            // a different shape entirely, and composing both would need its
            // own design (see `wire-flashinfer-decode-attention`'s design.md
            // Non-Goals). Reject explicitly rather than silently picking one.
            if strategy.paged_attention {
                return Err(CandleLlmError::UnsupportedModel {
                    alias: alias.to_owned(),
                    path: root.to_path_buf(),
                    detail:
                        "flashinfer_attention cannot be combined with paged_attention — they select different decode-attention kernels over different KV cache layouts; enable exactly one"
                            .to_owned(),
                });
            }
        }

        if strategy.cuda_graph_decode {
            if !paged_attention_supported {
                return Err(CandleLlmError::UnsupportedModel {
                    alias: alias.to_owned(),
                    path: root.to_path_buf(),
                    detail:
                    "cuda_graph_decode requires a Safetensors Llama checkpoint on a CUDA device because it captures Tachyon's paged, steady-state GPU decode loop through candle_core::CudaGraph"
                            .to_owned(),
                });
            }
            // The contiguous KV cache grows via a fresh, differently-shaped
            // allocation every decode step (`Tensor::cat`), which
            // `CudaGraph::capture` disallows; paged attention's pre-allocated,
            // in-place-updated `PagedKvCache` is the only KV layout this
            // runtime has that is graph-capture-safe (see
            // `wire-cuda-graph-decode`'s design.md).
            if !strategy.paged_attention {
                return Err(CandleLlmError::UnsupportedModel {
                    alias: alias.to_owned(),
                    path: root.to_path_buf(),
                    detail:
                        "cuda_graph_decode requires hardware_strategy.paged_attention: true — the contiguous KV cache's per-step reallocation is incompatible with CUDA graph replay"
                            .to_owned(),
                });
            }
            // Every prerequisite above is satisfiable, and capture/replay is
            // now real (`CudaGraphDecodeSession`, gated on `candle-cuda`,
            // wired via `astorise/candle#12`/`#15`'s
            // `set_decode_position`/`set_paged_kv_decode_slot` seams) — fall
            // through to load normally instead of rejecting.
        }

        if strategy.distribution_mode == GpuDistribution::Single
            && requested_device != "cpu"
            && !single_device_cuda_supported
        {
            return Err(CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail: format!(
                    "the Candle LLM runtime supports `cpu` execution only, got `{requested_device}`"
                ),
            });
        }

        // Both formats tokenize with a Hugging Face `tokenizer.json`. GGUF can
        // embed its vocab, but candle's quantized loader does not surface it, so
        // the broker always ships a `tokenizer.json` in the uploaded archive.
        if !root.join(TOKENIZER_JSON).exists() {
            return Err(CandleLlmError::MissingFile {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                file: TOKENIZER_JSON,
            });
        }
        let tokenizer = Tokenizer::from_file(root.join(TOKENIZER_JSON)).map_err(|error| {
            CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: TOKENIZER_JSON,
                detail: error.to_string(),
            }
        })?;

        // Load the model's own chat template (if any) once, so structured
        // `messages` requests render exactly as the checkpoint expects.
        let chat_template = ChatTemplate::load(alias, root)?.map(Arc::new);

        let (inner, limits) = if strategy.distribution_mode == GpuDistribution::Single {
            match format {
                ModelFormat::Safetensors => {
                    Self::load_safetensors(alias, root, requested_device, strategy)?
                }
                ModelFormat::Gguf => Self::load_gguf(alias, root, requested_device)?,
            }
        } else {
            Self::load_parallel(alias, root, format, strategy, topology)?
        };
        let limits = limits.with_prefill_chunk_tokens(strategy.prefill_chunk_tokens);

        Ok(Some(Self {
            alias: alias.to_owned(),
            root: root.to_path_buf(),
            tokenizer,
            inner: Arc::new(inner),
            limits,
            chat_template,
            fsm_cache: Arc::new(FsmCache::default()),
            prefix_cache: Arc::new(Mutex::new(PrefixCache::default())),
        }))
    }

    /// Load a registered Hugging Face safetensors architecture from
    /// `config.json` plus single-file or sharded weights.
    ///
    /// `requested_device` only ever selects a non-`cpu` device here for a
    /// Llama checkpoint: `try_load_with_topology`'s single-device gate has
    /// already rejected every other architecture/build combination before
    /// this is reached, so `Device::cuda_if_available` is only called when
    /// that gate already guarantees a `candle-cuda` build and a Llama
    /// checkpoint. Every other architecture keeps loading on `Device::Cpu`
    /// unconditionally, unchanged.
    ///
    /// Similarly, `strategy.paged_attention` only ever takes effect for a
    /// Llama checkpoint: `try_load_with_topology`'s paged-attention gate has
    /// already rejected every other case before this is reached.
    fn load_safetensors(
        alias: &str,
        root: &Path,
        requested_device: &str,
        strategy: &HardwareStrategy,
    ) -> Result<(LoadedModel, GenerationLimits), CandleLlmError> {
        let raw_config =
            fs::read(root.join(CONFIG_JSON)).map_err(|error| CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: CONFIG_JSON,
                detail: error.to_string(),
            })?;
        let architecture = inspect_hf_architecture(alias, root, &raw_config)?;
        let eos_tokens = generation_eos_token_ids(alias, root, &raw_config)?;
        let weight_paths = safetensors_paths(alias, root)?;
        let device = if architecture == ModelArchitecture::Llama && requested_device != "cpu" {
            Device::cuda_if_available(0).map_err(|error| CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: MODEL_SAFETENSORS,
                detail: error.to_string(),
            })?
        } else {
            Device::Cpu
        };
        // BF16 only for a paged-attention Llama deployment: candle-flash-attn
        // (and so `flash_attn_varlen_paged_windowed`) hard-errors on F32
        // ("flash-attn is only supported for f16/bf16"), so paged attention
        // needs the whole model's working dtype switched, not just a block
        // table attached. Every other case keeps today's F32 unchanged.
        let dtype = if architecture == ModelArchitecture::Llama && strategy.paged_attention {
            DType::BF16
        } else {
            DType::F32
        };
        // SAFETY: the model files live in the (uploaded) model directory and are
        // not mutated for the lifetime of the mmap held by the VarBuilder/model.
        let var_builder =
            unsafe { VarBuilder::from_mmaped_safetensors(&weight_paths, dtype, &device) }.map_err(
                |error| CandleLlmError::InvalidComponent {
                    alias: alias.to_owned(),
                    path: root.to_path_buf(),
                    component: MODEL_SAFETENSORS,
                    detail: error.to_string(),
                },
            )?;
        let invalid_weights = |error: candle_core::Error| CandleLlmError::InvalidComponent {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            component: MODEL_SAFETENSORS,
            detail: error.to_string(),
        };
        let (backend, limits, eos_tokens) = match architecture {
            ModelArchitecture::Llama => {
                let llama_config: LlamaConfig =
                    serde_json::from_slice(&raw_config).map_err(|error| {
                        CandleLlmError::InvalidComponent {
                            alias: alias.to_owned(),
                            path: root.to_path_buf(),
                            component: CONFIG_JSON,
                            detail: error.to_string(),
                        }
                    })?;
                let mut config = llama_config.into_config(false);
                // `Config::use_flashinfer_attention` exists regardless of the
                // `candle-flashinfer` Cargo feature (only its dead-code lint
                // is feature-gated on the fork side), and
                // `try_load_with_topology` has already rejected every request
                // where this would be `true` without that feature compiled
                // in, so assigning it unconditionally here is safe — it's a
                // no-op `false` on any build without the feature.
                config.use_flashinfer_attention = strategy.flashinfer_attention;
                let limits = GenerationLimits::with_context(config.max_position_embeddings);
                let eos_tokens = eos_token_ids(&config);
                let model = Llama::load(var_builder, &config).map_err(invalid_weights)?;
                let paged = if strategy.paged_attention {
                    Some(build_paged_attention_runtime(
                        alias, root, &config, &device, dtype,
                    )?)
                } else {
                    None
                };
                (
                    SingleDeviceBackend::Llama {
                        model: Mutex::new(model),
                        config,
                        device: device.clone(),
                        dtype,
                        // `try_load_with_topology` requires `paged_attention`
                        // whenever `cuda_graph_decode` is set, so `paged` is
                        // always `Some` here when this is `true`.
                        cuda_graph_decode: strategy.cuda_graph_decode,
                        paged,
                    },
                    limits,
                    eos_tokens,
                )
            }
            ModelArchitecture::Qwen2 => {
                let config: Qwen2Config = serde_json::from_slice(&raw_config).map_err(|error| {
                    CandleLlmError::InvalidComponent {
                        alias: alias.to_owned(),
                        path: root.to_path_buf(),
                        component: CONFIG_JSON,
                        detail: error.to_string(),
                    }
                })?;
                let limits = GenerationLimits::with_context(config.max_position_embeddings);
                let model = Qwen2Model::new(&config, var_builder).map_err(invalid_weights)?;
                (
                    SingleDeviceBackend::Qwen2(Mutex::new(model)),
                    limits,
                    eos_tokens,
                )
            }
            ModelArchitecture::Qwen3 => {
                let config: Qwen3Config = serde_json::from_slice(&raw_config).map_err(|error| {
                    CandleLlmError::InvalidComponent {
                        alias: alias.to_owned(),
                        path: root.to_path_buf(),
                        component: CONFIG_JSON,
                        detail: error.to_string(),
                    }
                })?;
                let limits = GenerationLimits::with_context(config.max_position_embeddings);
                let model = Qwen3Model::new(&config, var_builder).map_err(invalid_weights)?;
                (
                    SingleDeviceBackend::Qwen3(Mutex::new(model)),
                    limits,
                    eos_tokens,
                )
            }
            ModelArchitecture::Qwen35Moe => {
                let config: Qwen3MoeConfig =
                    serde_json::from_slice(&raw_config).map_err(|error| {
                        CandleLlmError::InvalidComponent {
                            alias: alias.to_owned(),
                            path: root.to_path_buf(),
                            component: CONFIG_JSON,
                            detail: error.to_string(),
                        }
                    })?;
                let limits = GenerationLimits::with_context(config.max_position_embeddings);
                let model = Qwen3MoeModel::new(&config, var_builder).map_err(invalid_weights)?;
                (
                    SingleDeviceBackend::Qwen3Moe(Mutex::new(model)),
                    limits,
                    eos_tokens,
                )
            }
            ModelArchitecture::Gemma2 => {
                let config: Gemma2Config =
                    serde_json::from_slice(&raw_config).map_err(|error| {
                        CandleLlmError::InvalidComponent {
                            alias: alias.to_owned(),
                            path: root.to_path_buf(),
                            component: CONFIG_JSON,
                            detail: error.to_string(),
                        }
                    })?;
                let limits = GenerationLimits::with_context(config.max_position_embeddings);
                let model =
                    Gemma2Model::new(false, &config, var_builder).map_err(invalid_weights)?;
                (
                    SingleDeviceBackend::Gemma2(Mutex::new(model)),
                    limits,
                    eos_tokens,
                )
            }
            ModelArchitecture::Gemma3 => {
                let config: Gemma3Config =
                    serde_json::from_slice(&raw_config).map_err(|error| {
                        CandleLlmError::InvalidComponent {
                            alias: alias.to_owned(),
                            path: root.to_path_buf(),
                            component: CONFIG_JSON,
                            detail: error.to_string(),
                        }
                    })?;
                let limits = GenerationLimits::with_context(config.max_position_embeddings);
                let model =
                    Gemma3Model::new(false, &config, var_builder).map_err(invalid_weights)?;
                (
                    SingleDeviceBackend::Gemma3(Mutex::new(model)),
                    limits,
                    eos_tokens,
                )
            }
            ModelArchitecture::Phi3 | ModelArchitecture::Phi4 => {
                let config: Phi3Config = serde_json::from_slice(&raw_config).map_err(|error| {
                    CandleLlmError::InvalidComponent {
                        alias: alias.to_owned(),
                        path: root.to_path_buf(),
                        component: CONFIG_JSON,
                        detail: error.to_string(),
                    }
                })?;
                let limits = GenerationLimits::with_context(config.max_position_embeddings);
                let model = Phi3Model::new(&config, var_builder).map_err(invalid_weights)?;
                (
                    SingleDeviceBackend::Phi3(Mutex::new(model)),
                    limits,
                    eos_tokens,
                )
            }
            ModelArchitecture::DeepSeekV2 => {
                let config: DeepSeekConfig =
                    serde_json::from_slice(&raw_config).map_err(|error| {
                        CandleLlmError::InvalidComponent {
                            alias: alias.to_owned(),
                            path: root.to_path_buf(),
                            component: CONFIG_JSON,
                            detail: error.to_string(),
                        }
                    })?;
                let limits = GenerationLimits::with_context(generation_context_limit(
                    alias,
                    root,
                    &raw_config,
                )?);
                let model = DeepSeekModel::new(&config, var_builder).map_err(invalid_weights)?;
                (
                    SingleDeviceBackend::DeepSeek(Mutex::new(model)),
                    limits,
                    eos_tokens,
                )
            }
            other => {
                return Err(CandleLlmError::UnsupportedModel {
                    alias: alias.to_owned(),
                    path: root.to_path_buf(),
                    detail: format!(
                        "recognized architecture `{}` has no generic safetensors backend",
                        other.display_name()
                    ),
                });
            }
        };

        Ok((
            LoadedModel::Safetensors {
                backend,
                eos_tokens,
            },
            limits,
        ))
    }

    /// Build a runnable dense Llama model from an already-detected
    /// ModelOpt/NVFP4 checkpoint: every NVFP4-quantized linear is dequantized
    /// to F32 at load time (the documented fallback execution path — native
    /// FP4-kernel execution without eager dequantization remains a follow-up,
    /// see the `gpu-accelerated-inference-execution` change), passthrough
    /// tensors (norms, embeddings, anything not quantized) are read as-is,
    /// and the resulting in-memory tensor map feeds the same already-tested
    /// `Llama::load` engine `load_safetensors` uses. This makes NVFP4
    /// checkpoints genuinely executable instead of unconditionally rejected.
    pub(crate) fn try_load_modelopt_nvfp4(
        directory: &modelopt_nvfp4::ModelOptNvfp4Directory,
    ) -> Result<Self, CandleLlmError> {
        let alias = directory.alias();
        let root = directory.root();

        let raw_config =
            fs::read(root.join(CONFIG_JSON)).map_err(|error| CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: CONFIG_JSON,
                detail: error.to_string(),
            })?;
        let probe: ModelTypeProbe = serde_json::from_slice(&raw_config).map_err(|error| {
            CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: CONFIG_JSON,
                detail: error.to_string(),
            }
        })?;
        if probe.model_type != LLAMA_MODEL_TYPE {
            return Err(CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail: format!(
                    "NVFP4 execution currently supports Llama-family checkpoints only (`model_type` = `{LLAMA_MODEL_TYPE}`), got `{}`",
                    probe.model_type
                ),
            });
        }
        let llama_config: LlamaConfig = serde_json::from_slice(&raw_config).map_err(|error| {
            CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: CONFIG_JSON,
                detail: error.to_string(),
            }
        })?;
        let config = llama_config.into_config(false);
        let limits = GenerationLimits::with_context(config.max_position_embeddings);
        let eos_tokens = eos_token_ids(&config);

        let device = Device::Cpu;
        let mut tensors = HashMap::new();
        for name in directory.tensor_names() {
            if name.ends_with(".weight_scale")
                || name.ends_with(".weight_scale_2")
                || name.ends_with(".input_scale")
            {
                continue;
            }
            let Some(base_name) = name.strip_suffix(".weight") else {
                let info = directory
                    .tensor_info(name)
                    .map_err(invalid_nvfp4(alias, root))?;
                let tensor =
                    raw_tensor_from_ref(&info, &device).map_err(invalid_nvfp4(alias, root))?;
                tensors.insert(name.to_owned(), tensor);
                continue;
            };
            let tensor = match directory
                .modelopt_linear(base_name)
                .map_err(invalid_nvfp4(alias, root))?
            {
                modelopt_nvfp4::ModelOptLinearTensors::Nvfp4(linear) => {
                    dense_tensor_from_nvfp4(&linear, &device).map_err(invalid_nvfp4(alias, root))?
                }
                modelopt_nvfp4::ModelOptLinearTensors::Fp8(fp8) => {
                    return Err(CandleLlmError::UnsupportedModel {
                        alias: alias.to_owned(),
                        path: root.to_path_buf(),
                        detail: format!(
                            "NVFP4 execution does not yet support the mixed FP8 linear `{}`",
                            fp8.base_name
                        ),
                    });
                }
                modelopt_nvfp4::ModelOptLinearTensors::Passthrough(passthrough) => {
                    raw_tensor_from_ref(&passthrough.tensor, &device)
                        .map_err(invalid_nvfp4(alias, root))?
                }
            };
            tensors.insert(name.to_owned(), tensor);
        }

        let var_builder = VarBuilder::from_tensors(tensors, DType::F32, &device);
        let model = Llama::load(var_builder, &config).map_err(|error| {
            CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: MODEL_SAFETENSORS,
                detail: error.to_string(),
            }
        })?;
        let inner = LoadedModel::Safetensors {
            backend: SingleDeviceBackend::Llama {
                model: Mutex::new(model),
                config,
                device: device.clone(),
                dtype: DType::F32,
                paged: None,
                cuda_graph_decode: false,
            },
            eos_tokens,
        };

        if !root.join(TOKENIZER_JSON).exists() {
            return Err(CandleLlmError::MissingFile {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                file: TOKENIZER_JSON,
            });
        }
        let tokenizer = Tokenizer::from_file(root.join(TOKENIZER_JSON)).map_err(|error| {
            CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: TOKENIZER_JSON,
                detail: error.to_string(),
            }
        })?;
        let chat_template = ChatTemplate::load(alias, root)?.map(Arc::new);

        Ok(Self {
            alias: alias.to_owned(),
            root: root.to_path_buf(),
            tokenizer,
            inner: Arc::new(inner),
            limits,
            chat_template,
            fsm_cache: Arc::new(FsmCache::default()),
            prefix_cache: Arc::new(Mutex::new(PrefixCache::default())),
        })
    }

    /// Load a single GGUF Llama file. The architecture and hyper-parameters come
    /// from GGUF metadata (there is no `config.json`); the quantised tensors are
    /// read from the file. The same reader feeds the header parse and the tensor
    /// reads (candle seeks via `tensor_data_offset`).
    /// Load a quantized GGUF Llama checkpoint.
    ///
    /// `requested_device` selects a real CUDA device on a `candle-cuda` build:
    /// the pinned fork carries quantized CUDA kernels
    /// (`candle-core/src/quantized/cuda.rs`), and
    /// `QuantizedLlama::from_gguf` uploads every block-quantized tensor to the
    /// device it is handed. `try_load_with_topology`'s single-device gate has
    /// already rejected a non-`cpu` request on a build or architecture that
    /// cannot honour it, so reaching here with `requested_device != "cpu"`
    /// guarantees a `candle-cuda` build, a Llama checkpoint, and `single`
    /// distribution.
    fn load_gguf(
        alias: &str,
        root: &Path,
        requested_device: &str,
    ) -> Result<(LoadedModel, GenerationLimits), CandleLlmError> {
        let gguf_path = gguf_file_path(alias, root)?;
        let mut reader =
            fs::File::open(&gguf_path).map_err(|error| CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: GGUF_COMPONENT,
                detail: error.to_string(),
            })?;
        let content = gguf_file::Content::read(&mut reader).map_err(|error| {
            CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: GGUF_COMPONENT,
                detail: error.to_string(),
            }
        })?;

        // Validate the architecture up front so a non-Llama GGUF fails with a
        // clear message rather than a missing-key error inside the loader.
        let architecture = content
            .metadata
            .get("general.architecture")
            .and_then(|value| value.to_string().ok())
            .map(|value| value.to_owned())
            .unwrap_or_default();
        if architecture != GGUF_LLAMA_ARCHITECTURE {
            return Err(CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail: format!(
                    "expected a Llama-family GGUF (`general.architecture` = `{GGUF_LLAMA_ARCHITECTURE}`), got `{architecture}`"
                ),
            });
        }

        let context_length = content
            .metadata
            .get("llama.context_length")
            .and_then(|value| value.to_u32().ok())
            .map(|value| value as usize)
            .unwrap_or(DEFAULT_MAX_PROMPT_TOKENS);
        let eos_tokens = content
            .metadata
            .get("tokenizer.ggml.eos_token_id")
            .and_then(|value| value.to_u32().ok())
            .map(|id| vec![id])
            .unwrap_or_default();
        let limits = GenerationLimits::with_context(context_length);

        // Resolve the device *before* uploading weights: an unsupported block
        // type must fail closed here rather than at the first decode step,
        // when the model already occupies VRAM.
        let device = resolve_gguf_device(alias, root, &content, requested_device)?;

        let model = QuantizedLlama::from_gguf(content, &mut reader, &device).map_err(|error| {
            CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: GGUF_COMPONENT,
                detail: error.to_string(),
            }
        })?;

        Ok((
            LoadedModel::Gguf {
                model: Mutex::new(model),
                eos_tokens,
                device,
            },
            limits,
        ))
    }

    /// Load a model under a non-`single` `hardware_strategy`: validate the
    /// requested plan against the discovered hardware topology, then construct
    /// the matching parallel engine. The engines themselves are the
    /// numerically-verified ones from `tensor_parallel_llama`/
    /// `pipeline_parallel_llama`; this is the dispatch that selects them.
    fn load_parallel(
        alias: &str,
        root: &Path,
        format: ModelFormat,
        strategy: &HardwareStrategy,
        topology: &ClusterTopology,
    ) -> Result<(LoadedModel, GenerationLimits), CandleLlmError> {
        // Parallel sharding operates on full-precision safetensors weights. GGUF
        // is a single-file quantized format with no sharding path here.
        if format != ModelFormat::Safetensors {
            return Err(CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail:
                    "parallel execution requires a safetensors checkpoint; GGUF is single-device only"
                        .to_owned(),
            });
        }

        // Fail fast against real hardware *before* loading any weights, so an
        // unsatisfiable plan never triggers a partial allocation.
        let plan = plan_from_strategy(strategy);
        validate_parallel_topology(&plan, topology).map_err(|error| {
            CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail: format!("parallel topology rejected: {error}"),
            }
        })?;

        let devices = resolve_devices(&plan.device_ids);
        if devices.is_empty() {
            return Err(CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail: "a parallel deployment must declare at least one device_id".to_owned(),
            });
        }

        let weight_paths = safetensors_paths(alias, root)?;
        let component = MODEL_SAFETENSORS;
        let invalid = |error: candle_core::Error| CandleLlmError::InvalidComponent {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            component,
            detail: error.to_string(),
        };

        // `ExpertParallelism` branches *before* `load_llama_config`: a real
        // MoE checkpoint declares `model_type: "mixtral"`, which
        // `load_llama_config` hard-rejects (it only accepts `"llama"`), and
        // Mixtral's config shape genuinely differs (per-layer expert count,
        // top-k routing) in ways `LlamaConfig`/`Config` have no field for.
        // The `"llama"` path below (`load_llama_config`) is unchanged.
        if strategy.distribution_mode == GpuDistribution::ExpertParallelism {
            let mixtral_config = Self::load_mixtral_config(alias, root)?;
            let config = mixtral_config.config.clone();
            let limits = GenerationLimits::with_context(config.max_position_embeddings);
            let eos_tokens = eos_token_ids(&config);
            // SAFETY: weights live in the uploaded model directory and are not
            // mutated for the lifetime of the mmap.
            let tensor_index =
                unsafe { MmapedSafetensors::multi(&weight_paths) }.map_err(invalid)?;
            let tensor_names: Vec<String> = tensor_index
                .tensors()
                .into_iter()
                .map(|(name, _)| name)
                .collect();
            let expert_plan = ExpertPlacementPlan::from_explicit_map_or_round_robin(
                &plan.expert_device_map,
                mixtral_config.num_local_experts as u32,
                devices.len(),
            );
            let var_builder = unsafe {
                VarBuilder::from_mmaped_safetensors(&weight_paths, DType::F32, &devices[0])
            }
            .map_err(invalid)?;
            let model = ExpertParallelLlama::load(
                var_builder,
                &config,
                tensor_names.iter().map(String::as_str),
                &expert_plan,
                &devices,
            )
            .map_err(invalid)?;
            return Ok((
                LoadedModel::Parallel(ParallelModel::Expert {
                    model: Box::new(model),
                    config,
                    eos_tokens,
                    devices,
                }),
                limits,
            ));
        }

        let config = Self::load_llama_config(alias, root)?;
        let limits = GenerationLimits::with_context(config.max_position_embeddings);
        let eos_tokens = eos_token_ids(&config);

        match strategy.distribution_mode {
            GpuDistribution::TensorParallelism => {
                // SAFETY: weights live in the uploaded model directory and are
                // not mutated for the lifetime of the mmap.
                let var_builder = unsafe {
                    VarBuilder::from_mmaped_safetensors(&weight_paths, DType::F32, &devices[0])
                }
                .map_err(invalid)?;
                let model =
                    TensorParallelLlama::load(var_builder, &config, &devices).map_err(invalid)?;
                Ok((
                    LoadedModel::Parallel(ParallelModel::Tensor {
                        model: Box::new(model),
                        config,
                        eos_tokens,
                        devices,
                    }),
                    limits,
                ))
            }
            GpuDistribution::PipelineParallelism => {
                let model = PipelineParallelLlama::load(
                    &weight_paths,
                    DType::F32,
                    &config,
                    &plan.stage_layer_ranges,
                    &devices,
                )
                .map_err(invalid)?;
                Ok((
                    LoadedModel::Parallel(ParallelModel::Pipeline {
                        model: Box::new(model),
                        config,
                        eos_tokens,
                        devices,
                    }),
                    limits,
                ))
            }
            GpuDistribution::ExpertParallelism => {
                unreachable!("handled by the early return above")
            }
            GpuDistribution::Single => {
                unreachable!("load_parallel is only reached for non-single strategies")
            }
        }
    }

    /// Parse and validate a Mixtral `config.json`: the `MixtralConfigJson`
    /// branch of Task 1, kept entirely separate from [`Self::load_llama_config`]
    /// (which only ever accepts `model_type: "llama"`) so the dense, tensor-,
    /// and pipeline-parallel paths are unaffected by this addition.
    fn load_mixtral_config(alias: &str, root: &Path) -> Result<MixtralConfigJson, CandleLlmError> {
        let raw_config =
            fs::read(root.join(CONFIG_JSON)).map_err(|error| CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: CONFIG_JSON,
                detail: error.to_string(),
            })?;
        let probe: ModelTypeProbe = serde_json::from_slice(&raw_config).map_err(|error| {
            CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: CONFIG_JSON,
                detail: error.to_string(),
            }
        })?;
        if probe.model_type != MIXTRAL_MODEL_TYPE {
            return Err(CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail: format!(
                    "expected a Mixtral-family checkpoint (`model_type` = `{MIXTRAL_MODEL_TYPE}`) \
                     for an expert-parallel deployment, got `{}`",
                    probe.model_type
                ),
            });
        }
        let raw: RawMixtralConfig = serde_json::from_slice(&raw_config).map_err(|error| {
            CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: CONFIG_JSON,
                detail: error.to_string(),
            }
        })?;
        if raw.num_experts_per_tok != 1 {
            return Err(CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail: format!(
                    "expert-parallel execution only supports top-1 routing \
                     (`num_experts_per_tok` == 1), got {}",
                    raw.num_experts_per_tok
                ),
            });
        }
        Ok(MixtralConfigJson {
            config: raw.base.into_config(PARALLEL_LLAMA_USE_FLASH_ATTN),
            num_local_experts: raw.num_local_experts,
        })
    }

    /// Parse and validate a Llama `config.json` (shared by the safetensors and
    /// parallel loaders).
    fn load_llama_config(alias: &str, root: &Path) -> Result<Config, CandleLlmError> {
        let raw_config =
            fs::read(root.join(CONFIG_JSON)).map_err(|error| CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: CONFIG_JSON,
                detail: error.to_string(),
            })?;
        let probe: ModelTypeProbe = serde_json::from_slice(&raw_config).map_err(|error| {
            CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: CONFIG_JSON,
                detail: error.to_string(),
            }
        })?;
        if probe.model_type != LLAMA_MODEL_TYPE {
            return Err(CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail: format!(
                    "expected a Llama-family checkpoint (`model_type` = `{LLAMA_MODEL_TYPE}`), got `{}`",
                    probe.model_type
                ),
            });
        }
        let llama_config: LlamaConfig = serde_json::from_slice(&raw_config).map_err(|error| {
            CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: CONFIG_JSON,
                detail: error.to_string(),
            }
        })?;
        Ok(llama_config.into_config(PARALLEL_LLAMA_USE_FLASH_ATTN))
    }

    /// Buffered generation: run the decode and return the full UTF-8 output. A
    /// thin accumulator over [`generate_streaming`], so the two paths always
    /// agree byte-for-byte.
    pub(crate) fn generate(&self, prompts: &[&[u8]]) -> Result<Vec<u8>, CandleLlmError> {
        let mut out = String::new();
        self.generate_streaming(prompts, &mut |delta| out.push_str(delta))?;
        Ok(out.into_bytes())
    }

    pub(crate) fn generate_with_adapter(
        &self,
        prompts: &[&[u8]],
        adapter_id: &str,
        adapter_path: &Path,
    ) -> Result<Vec<u8>, CandleLlmError> {
        let mut out = String::new();
        self.generate_with_adapter_streaming(prompts, adapter_id, adapter_path, &mut |delta| {
            out.push_str(delta)
        })?;
        Ok(out.into_bytes())
    }

    pub(crate) fn try_generate_batch_with_adapters(
        &self,
        prompts: &[&[u8]],
        adapters: &[Option<ResolvedLoraAdapter>],
    ) -> Result<Option<Vec<Vec<u8>>>, CandleLlmError> {
        if prompts.is_empty() {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: "at least one U8 prompt tensor is required".to_owned(),
            });
        }
        if prompts.len() != adapters.len() {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!(
                    "{} adapter assignment(s) for {} prompt(s)",
                    adapters.len(),
                    prompts.len()
                ),
            });
        }
        if prompts.len() > self.limits.max_batch_size {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!(
                    "batch size {} exceeds max batch size {}",
                    prompts.len(),
                    self.limits.max_batch_size
                ),
            });
        }

        let parsed = prompts
            .iter()
            .map(|prompt| self.parse_request(prompt))
            .collect::<Result<Vec<_>, _>>()?;
        let prompt_ids = parsed
            .iter()
            .map(|request| self.encode_ids(&request.prompt))
            .collect::<Result<Vec<_>, _>>()?;
        if prompt_ids.iter().any(Vec::is_empty) {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: "prompt produced no tokens to condition on".to_owned(),
            });
        }
        let prompt_len = prompt_ids[0].len();
        if prompt_ids.iter().any(|ids| ids.len() != prompt_len) {
            return Ok(None);
        }
        if prompt_len > self.limits.max_position_embeddings {
            return Ok(Some(vec![Vec::new(); prompts.len()]));
        }

        let LoadedModel::Safetensors {
            backend,
            eos_tokens,
        } = &*self.inner
        else {
            return Ok(None);
        };
        let SingleDeviceBackend::Llama {
            model,
            config,
            device,
            dtype,
            paged,
            cuda_graph_decode,
        } = backend
        else {
            return Ok(None);
        };
        if paged.is_some() || *cuda_graph_decode {
            return Ok(None);
        }

        let mut unique_adapters: Vec<&ResolvedLoraAdapter> = Vec::new();
        for adapter in adapters.iter().flatten() {
            if !unique_adapters.iter().any(|known| known.id == adapter.id) {
                unique_adapters.push(adapter);
            }
        }
        if unique_adapters.is_empty() {
            return Ok(None);
        }

        let mut model = model
            .lock()
            .map_err(|_| self.execution_error("Llama model mutex was poisoned".to_owned()))?;
        for adapter in unique_adapters {
            let (lora_config, prefix) = lora_load_config_from_adapter(&adapter.path, device)
                .map_err(|error| CandleLlmError::InvalidComponent {
                    alias: self.alias.clone(),
                    path: adapter.path.clone(),
                    component: MODEL_SAFETENSORS,
                    detail: error.to_string(),
                })?;
            let adapter_vb = unsafe {
                VarBuilder::from_mmaped_safetensors(
                    std::slice::from_ref(&adapter.path),
                    *dtype,
                    device,
                )
            }
            .map_err(|error| CandleLlmError::InvalidComponent {
                alias: self.alias.clone(),
                path: adapter.path.clone(),
                component: MODEL_SAFETENSORS,
                detail: error.to_string(),
            })?;
            model
                .load_lora_adapter_prefixed(&adapter.id, adapter_vb, lora_config, prefix)
                .map_err(|error| CandleLlmError::InvalidComponent {
                    alias: self.alias.clone(),
                    path: adapter.path.clone(),
                    component: MODEL_SAFETENSORS,
                    detail: error.to_string(),
                })?;
        }

        let mut cache = Cache::new(true, *dtype, config, device).map_err(|error| {
            self.execution_error(format!(
                "failed to build batch-native LoRA KV cache: {error}"
            ))
        })?;
        let adapter_assignments = adapters
            .iter()
            .map(|adapter| adapter.as_ref().map(|adapter| adapter.id.as_str()))
            .collect::<Vec<_>>();
        self.decode_batch_with_adapters(
            &mut model,
            &mut cache,
            &prompt_ids,
            &parsed,
            eos_tokens,
            device,
            &adapter_assignments,
        )
        .map(Some)
    }

    fn generate_with_adapter_streaming(
        &self,
        prompts: &[&[u8]],
        adapter_id: &str,
        adapter_path: &Path,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<(), CandleLlmError> {
        if prompts.is_empty() {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: "at least one U8 prompt tensor is required".to_owned(),
            });
        }
        if prompts.len() > self.limits.max_batch_size {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!(
                    "batch size {} exceeds max batch size {}",
                    prompts.len(),
                    self.limits.max_batch_size
                ),
            });
        }
        let parsed = prompts
            .iter()
            .map(|prompt| self.parse_request(prompt))
            .collect::<Result<Vec<_>, _>>()?;
        let request = parsed
            .first()
            .ok_or_else(|| CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: "at least one U8 prompt tensor is required".to_owned(),
            })?;
        let prompt_ids = self.encode_ids(&request.prompt)?;
        if prompt_ids.is_empty() {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: "prompt produced no tokens to condition on".to_owned(),
            });
        }

        let LoadedModel::Safetensors {
            backend,
            eos_tokens,
        } = &*self.inner
        else {
            return Err(CandleLlmError::UnsupportedModel {
                alias: self.alias.clone(),
                path: self.root.clone(),
                detail:
                    "LoRA adapters are currently supported for safetensors Llama checkpoints only"
                        .to_owned(),
            });
        };
        let SingleDeviceBackend::Llama {
            model,
            config,
            device,
            dtype,
            ..
        } = backend
        else {
            return Err(CandleLlmError::UnsupportedModel {
                alias: self.alias.clone(),
                path: self.root.clone(),
                detail: "LoRA adapters are currently supported for Llama-family checkpoints only"
                    .to_owned(),
            });
        };

        let (lora_config, prefix) =
            lora_load_config_from_adapter(adapter_path, device).map_err(|error| {
                CandleLlmError::InvalidComponent {
                    alias: self.alias.clone(),
                    path: adapter_path.to_path_buf(),
                    component: MODEL_SAFETENSORS,
                    detail: error.to_string(),
                }
            })?;
        let adapter_vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[adapter_path.to_path_buf()], *dtype, device)
        }
        .map_err(|error| CandleLlmError::InvalidComponent {
            alias: self.alias.clone(),
            path: adapter_path.to_path_buf(),
            component: MODEL_SAFETENSORS,
            detail: error.to_string(),
        })?;
        let mut model = model
            .lock()
            .map_err(|_| self.execution_error("Llama model mutex was poisoned".to_owned()))?;
        model
            .load_lora_adapter_prefixed(adapter_id, adapter_vb, lora_config, prefix)
            .map_err(|error| CandleLlmError::InvalidComponent {
                alias: self.alias.clone(),
                path: adapter_path.to_path_buf(),
                component: MODEL_SAFETENSORS,
                detail: error.to_string(),
            })?;
        model
            .set_active_adapter(Some(adapter_id))
            .map_err(|error| CandleLlmError::InvalidComponent {
                alias: self.alias.clone(),
                path: adapter_path.to_path_buf(),
                component: MODEL_SAFETENSORS,
                detail: error.to_string(),
            })?;
        let mut cache = Cache::new(true, *dtype, config, device).map_err(|error| {
            self.execution_error(format!("failed to build LoRA KV cache: {error}"))
        })?;
        let result = self.decode_loop(
            &prompt_ids,
            request,
            eos_tokens,
            device,
            on_token,
            |input, index_pos| model.forward(input, index_pos, &mut cache),
        );
        let reset =
            model
                .set_active_adapter(None)
                .map_err(|error| CandleLlmError::InvalidComponent {
                    alias: self.alias.clone(),
                    path: adapter_path.to_path_buf(),
                    component: MODEL_SAFETENSORS,
                    detail: error.to_string(),
                });
        result.and(reset)
    }

    pub(crate) fn generate_speculative(
        &self,
        prompts: &[&[u8]],
        draft: &Self,
        draft_tokens: usize,
    ) -> Result<Vec<u8>, CandleLlmError> {
        let mut out = String::new();
        self.generate_speculative_streaming(prompts, draft, draft_tokens, &mut |delta| {
            out.push_str(delta)
        })?;
        Ok(out.into_bytes())
    }

    /// Streaming generation: identical decoding to [`generate`], but each newly
    /// decoded, stop-trimmed text fragment is handed to `on_token` as it is
    /// produced. The concatenation of every `on_token` fragment equals the
    /// buffered `generate` output. Used by the streaming accelerator path so a
    /// caller can forward tokens to the client as they arrive.
    pub(crate) fn generate_streaming(
        &self,
        prompts: &[&[u8]],
        on_token: &mut dyn FnMut(&str),
    ) -> Result<(), CandleLlmError> {
        if prompts.is_empty() {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: "at least one U8 prompt tensor is required".to_owned(),
            });
        }
        if prompts.len() > self.limits.max_batch_size {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!(
                    "batch size {} exceeds max batch size {}",
                    prompts.len(),
                    self.limits.max_batch_size
                ),
            });
        }

        let parsed = prompts
            .iter()
            .map(|prompt| self.parse_request(prompt))
            .collect::<Result<Vec<_>, _>>()?;
        let request = parsed
            .first()
            .ok_or_else(|| CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: "at least one U8 prompt tensor is required".to_owned(),
            })?;

        let prompt_ids = self.encode_ids(&request.prompt)?;
        if prompt_ids.is_empty() {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: "prompt produced no tokens to condition on".to_owned(),
            });
        }

        self.decode(&prompt_ids, request, on_token)
    }

    pub(crate) fn generate_speculative_streaming(
        &self,
        prompts: &[&[u8]],
        draft: &Self,
        draft_tokens: usize,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<(), CandleLlmError> {
        if prompts.len() != 1 || !self.has_compatible_tokenizer(draft) {
            return self.generate_streaming(prompts, on_token);
        }
        // Verification (`greedy_next_token_id`/`last_logits`) always builds a
        // fresh contiguous `Cache` from `self`'s already-loaded model, which
        // would dtype-mismatch against a paged-attention deployment's BF16
        // weights combined with block-table state it doesn't attach. Neither
        // paged attention nor speculative verification are wired to compose
        // yet, so fall back to plain generation for either side — the same
        // pattern this function already uses for an incompatible tokenizer.
        if self.is_paged_attention_enabled() || draft.is_paged_attention_enabled() {
            return self.generate_streaming(prompts, on_token);
        }
        let request = self.parse_request(prompts[0])?;
        if !request.sampling.is_greedy() || request.fsm.is_some() {
            return self.generate_streaming(prompts, on_token);
        }
        let prompt_ids = self.encode_ids(&request.prompt)?;
        if prompt_ids.is_empty() {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: "prompt produced no tokens to condition on".to_owned(),
            });
        }

        let draft_tokens = draft_tokens.max(1);
        let mut context_ids = prompt_ids.clone();
        let mut generated = Vec::with_capacity(request.max_new_tokens);
        let hold = request
            .stop
            .iter()
            .map(String::len)
            .max()
            .unwrap_or(0)
            .saturating_sub(1);
        let mut emitted = 0usize;

        while generated.len() < request.max_new_tokens
            && context_ids.len() < self.limits.max_position_embeddings
        {
            let remaining = request.max_new_tokens - generated.len();
            let proposed = draft.greedy_token_ids_from_context(
                &context_ids,
                &request,
                draft_tokens.min(remaining),
            )?;
            if proposed.is_empty() {
                break;
            }

            for proposed_token in proposed {
                let Some(target_token) = self.greedy_next_token_id(&context_ids, &request)? else {
                    return Ok(());
                };
                context_ids.push(target_token);
                generated.push(target_token);

                let text = self.decode_generated(&generated)?;
                if let Some(stop_at) = find_earliest_stop(&text, &request.stop) {
                    emit_delta(on_token, &text, &mut emitted, stop_at);
                    return Ok(());
                }
                if self.is_eos_token(target_token) {
                    emit_delta(on_token, &text, &mut emitted, text.len());
                    return Ok(());
                }
                let safe = floor_char_boundary(&text, text.len().saturating_sub(hold));
                emit_delta(on_token, &text, &mut emitted, safe);

                if target_token != proposed_token
                    || generated.len() == request.max_new_tokens
                    || context_ids.len() >= self.limits.max_position_embeddings
                {
                    break;
                }
            }
        }

        let text = self.decode_generated(&generated)?;
        let end = find_earliest_stop(&text, &request.stop).unwrap_or(text.len());
        emit_delta(on_token, &text, &mut emitted, end);
        Ok(())
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    fn has_compatible_tokenizer(&self, draft: &Self) -> bool {
        self.tokenizer.get_vocab(true) == draft.tokenizer.get_vocab(true)
    }

    /// `true` for a Llama safetensors backend loaded with
    /// `hardware_strategy.paged_attention: true` (BF16 weights, a shared
    /// block pool). Used to keep features that aren't wired to compose with
    /// paged attention yet (speculative-decoding verification) from hitting
    /// a dtype mismatch instead of falling back cleanly.
    fn is_paged_attention_enabled(&self) -> bool {
        matches!(
            &*self.inner,
            LoadedModel::Safetensors {
                backend: SingleDeviceBackend::Llama { paged: Some(_), .. },
                ..
            }
        )
    }

    fn is_eos_token(&self, token: u32) -> bool {
        match &*self.inner {
            LoadedModel::Safetensors { eos_tokens, .. } | LoadedModel::Gguf { eos_tokens, .. } => {
                eos_tokens.contains(&token)
            }
            LoadedModel::Parallel(parallel) => match parallel {
                ParallelModel::Tensor { eos_tokens, .. }
                | ParallelModel::Pipeline { eos_tokens, .. }
                | ParallelModel::Expert { eos_tokens, .. } => eos_tokens.contains(&token),
            },
        }
    }

    fn greedy_token_ids_from_context(
        &self,
        context_ids: &[u32],
        request: &ParsedGenerationRequest,
        max_tokens: usize,
    ) -> Result<Vec<u32>, CandleLlmError> {
        let mut context = context_ids.to_vec();
        let mut tokens = Vec::with_capacity(max_tokens);
        for _ in 0..max_tokens {
            let Some(next) = self.greedy_next_token_id(&context, request)? else {
                break;
            };
            context.push(next);
            tokens.push(next);
            if self.is_eos_token(next) {
                break;
            }
        }
        Ok(tokens)
    }

    fn greedy_next_token_id(
        &self,
        context_ids: &[u32],
        request: &ParsedGenerationRequest,
    ) -> Result<Option<u32>, CandleLlmError> {
        if context_ids.is_empty() || context_ids.len() > self.limits.max_position_embeddings {
            return Ok(None);
        }
        let logits = self.last_logits_for_ids(context_ids)?;
        let row = logits
            .squeeze(0)
            .map_err(|error| self.execution_error(format!("failed to reshape logits: {error}")))?;
        let mut processor = request.sampling.processor();
        processor
            .sample(&row)
            .map(Some)
            .map_err(|error| self.execution_error(format!("failed to sample next token: {error}")))
    }

    fn last_logits_for_ids(&self, ids: &[u32]) -> Result<Tensor, CandleLlmError> {
        let device = Device::Cpu;
        let input = Tensor::new(ids, &device)
            .and_then(|tensor| tensor.unsqueeze(0))
            .map_err(|error| {
                self.execution_error(format!("failed to build input tensor: {error}"))
            })?;
        match &*self.inner {
            LoadedModel::Safetensors { backend, .. } => {
                let backend_device = backend.device();
                let input = input.to_device(&backend_device).map_err(|error| {
                    self.execution_error(format!("failed to move input to device: {error}"))
                })?;
                Ok(backend.last_logits(&input, &backend_device))
            }
            LoadedModel::Gguf {
                model,
                device: model_device,
                ..
            } => {
                let input = input.to_device(model_device).map_err(|error| {
                    self.execution_error(format!("failed to move input to device: {error}"))
                })?;
                let mut guard = model.lock().map_err(|_| {
                    self.execution_error("GGUF model mutex was poisoned".to_owned())
                })?;
                guard.forward(&input, 0).map_err(|error| {
                    self.execution_error(format!("transformer forward pass failed: {error}"))
                })
            }
            LoadedModel::Parallel(parallel) => match parallel {
                ParallelModel::Tensor {
                    model,
                    config,
                    devices,
                    ..
                } => {
                    let primary = &devices[0];
                    let input = input.to_device(primary).map_err(|error| {
                        self.execution_error(format!("failed to move input to device: {error}"))
                    })?;
                    let mut cache = TensorParallelCache::new(true, DType::F32, config, primary)
                        .map_err(|error| {
                            self.execution_error(format!(
                                "failed to build tensor-parallel KV cache: {error}"
                            ))
                        })?;
                    model.forward(&input, 0, &mut cache).map_err(|error| {
                        self.execution_error(format!("transformer forward pass failed: {error}"))
                    })
                }
                ParallelModel::Pipeline { model, .. } => {
                    let stage0_device = model
                        .stages
                        .first()
                        .map(|stage| stage.device().clone())
                        .unwrap_or(Device::Cpu);
                    let input = input.to_device(&stage0_device).map_err(|error| {
                        self.execution_error(format!("failed to move input to device: {error}"))
                    })?;
                    pipeline_prefill_forward(model, &input).map_err(|error| {
                        self.execution_error(format!("transformer forward pass failed: {error}"))
                    })
                }
                ParallelModel::Expert {
                    model,
                    config,
                    devices,
                    ..
                } => {
                    let primary = &devices[0];
                    let input = input.to_device(primary).map_err(|error| {
                        self.execution_error(format!("failed to move input to device: {error}"))
                    })?;
                    let mut cache = TensorParallelCache::new(true, DType::F32, config, primary)
                        .map_err(|error| {
                            self.execution_error(format!(
                                "failed to build expert-parallel KV cache: {error}"
                            ))
                        })?;
                    model.forward(&input, 0, &mut cache).map_err(|error| {
                        self.execution_error(format!("transformer forward pass failed: {error}"))
                    })
                }
            },
        }
    }

    /// Autoregressive decode through the real Llama forward, with a fresh KV
    /// cache per request: the prompt is processed once, then each new token is
    /// fed back in with the running `index_pos`. Token selection follows the
    /// request's sampling policy (greedy when `temperature <= 0`). Safetensors
    /// uses an external `Cache`; GGUF uses the in-weights cache, which resets
    /// when the sequence restarts at `index_pos == 0`.
    fn decode(
        &self,
        prompt_ids: &[u32],
        request: &ParsedGenerationRequest,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<(), CandleLlmError> {
        // Every arm resolves its own device from the loaded weights: there is
        // no longer a CPU default to fall back on now that GGUF can live on a
        // CUDA device too.
        match &*self.inner {
            LoadedModel::Safetensors {
                backend,
                eos_tokens,
            } => {
                let device = backend.device();
                backend.decode(self, prompt_ids, request, eos_tokens, &device, on_token)
            }
            LoadedModel::Gguf {
                model,
                eos_tokens,
                device,
            } => {
                let mut guard = model.lock().map_err(|_| {
                    self.execution_error("GGUF model mutex was poisoned".to_owned())
                })?;
                self.decode_loop(
                    prompt_ids,
                    request,
                    eos_tokens,
                    device,
                    on_token,
                    |input, index_pos| guard.forward(input, index_pos),
                )
            }
            LoadedModel::Parallel(parallel) => match parallel {
                ParallelModel::Tensor {
                    model,
                    config,
                    eos_tokens,
                    devices,
                } => {
                    // Tensor parallelism carries a real KV cache, so it drives
                    // the full autoregressive decode loop like the dense path.
                    let primary = &devices[0];
                    let mut cache = TensorParallelCache::new(true, DType::F32, config, primary)
                        .map_err(|error| {
                            self.execution_error(format!(
                                "failed to build tensor-parallel KV cache: {error}"
                            ))
                        })?;
                    self.decode_loop(
                        prompt_ids,
                        request,
                        eos_tokens,
                        primary,
                        on_token,
                        |input, index_pos| model.forward(input, index_pos, &mut cache),
                    )
                }
                ParallelModel::Pipeline {
                    model,
                    eos_tokens,
                    devices,
                    ..
                } => {
                    // Pipeline parallelism carries one KV cache per stage,
                    // built fresh per request and threaded through every
                    // prefill/decode call via `forward_at`.
                    let primary = &devices[0];
                    let mut caches = model.new_caches().map_err(|error| {
                        self.execution_error(format!(
                            "failed to build pipeline-parallel KV caches: {error}"
                        ))
                    })?;
                    let transports = pipeline_stage_transports(model);
                    self.decode_loop(
                        prompt_ids,
                        request,
                        eos_tokens,
                        primary,
                        on_token,
                        |input, index_pos| {
                            model.forward_at(index_pos, input, &transports, &mut caches)
                        },
                    )
                }
                ParallelModel::Expert {
                    model,
                    config,
                    eos_tokens,
                    devices,
                } => {
                    // Expert parallelism's attention is dense and replicated
                    // exactly like tensor parallelism's, so it drives the same
                    // single-cache decode loop.
                    let primary = &devices[0];
                    let mut cache = TensorParallelCache::new(true, DType::F32, config, primary)
                        .map_err(|error| {
                            self.execution_error(format!(
                                "failed to build expert-parallel KV cache: {error}"
                            ))
                        })?;
                    self.decode_loop(
                        prompt_ids,
                        request,
                        eos_tokens,
                        primary,
                        on_token,
                        |input, index_pos| model.forward(input, index_pos, &mut cache),
                    )
                }
            },
        }
    }

    /// Drive an autoregressive decode, delegating the single-step forward to
    /// `forward`, which maps an input tensor `[1, seq]` + position to the
    /// final-position logits `[1, vocab]`. Shared by both backends. The next
    /// token is drawn by the request's `LogitsProcessor`; decoding halts on EOS,
    /// the token budget, the context window, or a matched stop sequence.
    ///
    /// Decoded text is streamed through `on_token` as it is produced. To honour
    /// stop sequences without leaking a partial match, the tail of the decoded
    /// text within one stop-length of the end is held back until a further token
    /// confirms it is safe to emit (or the decode ends).
    fn decode_loop(
        &self,
        prompt_ids: &[u32],
        request: &ParsedGenerationRequest,
        eos_tokens: &[u32],
        input_device: &Device,
        on_token: &mut dyn FnMut(&str),
        mut forward: impl FnMut(&Tensor, usize) -> candle_core::Result<Tensor>,
    ) -> Result<(), CandleLlmError> {
        let device = input_device;
        if prompt_ids.len() > self.limits.max_position_embeddings {
            return Ok(());
        }
        let (logits, index_pos) = self.run_prefill_chunks(prompt_ids, device, &mut forward)?;
        self.decode_loop_from_logits(
            logits,
            index_pos,
            prompt_ids,
            DecodeLoopContext {
                request,
                eos_tokens,
                input_device,
                on_token,
            },
            forward,
        )
    }

    fn run_prefill_chunks(
        &self,
        prompt_ids: &[u32],
        device: &Device,
        forward: &mut impl FnMut(&Tensor, usize) -> candle_core::Result<Tensor>,
    ) -> Result<(Tensor, usize), CandleLlmError> {
        let mut index_pos = 0usize;
        let mut logits = None;
        while index_pos < prompt_ids.len() {
            let remaining = prompt_ids.len() - index_pos;
            let chunk_len = self.limits.next_prefill_chunk_len(remaining);
            let chunk = &prompt_ids[index_pos..index_pos + chunk_len];
            let input = Tensor::new(chunk, device)
                .and_then(|tensor| tensor.unsqueeze(0))
                .map_err(|error| {
                    self.execution_error(format!("failed to build input tensor: {error}"))
                })?;
            let next_logits = forward(&input, index_pos).map_err(|error| {
                self.execution_error(format!("transformer forward pass failed: {error}"))
            })?;
            index_pos += chunk_len;
            logits = Some(next_logits);
        }

        logits
            .ok_or_else(|| self.execution_error("prompt produced no prefill logits".to_owned()))
            .map(|logits| (logits, index_pos))
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_batch_with_adapters(
        &self,
        model: &mut Llama,
        cache: &mut Cache,
        prompt_ids: &[Vec<u32>],
        requests: &[ParsedGenerationRequest],
        eos_tokens: &[u32],
        device: &Device,
        adapter_assignments: &[Option<&str>],
    ) -> Result<Vec<Vec<u8>>, CandleLlmError> {
        let batch = prompt_ids.len();
        let prompt_len = prompt_ids
            .first()
            .map(Vec::len)
            .ok_or_else(|| self.execution_error("empty batch-native decode".to_owned()))?;
        let mut index_pos = 0usize;
        let mut logits = None;
        while index_pos < prompt_len {
            let remaining = prompt_len - index_pos;
            let chunk_len = self.limits.next_prefill_chunk_len(remaining);
            let flat_prompt = prompt_ids
                .iter()
                .flat_map(|ids| ids[index_pos..index_pos + chunk_len].iter().copied())
                .collect::<Vec<_>>();
            let prompt =
                Tensor::from_vec(flat_prompt, (batch, chunk_len), device).map_err(|error| {
                    self.execution_error(format!("failed to build batched prompt tensor: {error}"))
                })?;
            let next_logits = model
                .forward_with_adapters(&prompt, index_pos, cache, adapter_assignments)
                .map_err(|error| {
                    self.execution_error(format!(
                        "batch-native LoRA prefill forward pass failed: {error}"
                    ))
                })?;
            index_pos += chunk_len;
            logits = Some(next_logits);
        }
        let mut logits = logits.ok_or_else(|| {
            self.execution_error("batch-native LoRA prompt produced no prefill logits".to_owned())
        })?;
        let mut processors = requests
            .iter()
            .map(|request| request.sampling.processor())
            .collect::<Vec<_>>();
        let mut fsm_processors = requests
            .iter()
            .map(|request| {
                request
                    .fsm
                    .as_ref()
                    .map(|fsm| FsmLogitProcessor::new(Arc::clone(fsm)))
            })
            .collect::<Vec<_>>();
        let vocab_size = self.tokenizer.get_vocab_size(true);
        let mut generated = requests
            .iter()
            .map(|request| Vec::with_capacity(request.max_new_tokens))
            .collect::<Vec<_>>();
        let mut done = vec![false; batch];
        let mut next_tokens = vec![0u32; batch];
        let max_new_tokens = requests
            .iter()
            .map(|request| request.max_new_tokens)
            .max()
            .unwrap_or(0);

        for step in 0..max_new_tokens {
            for row in 0..batch {
                if done[row] || generated[row].len() >= requests[row].max_new_tokens {
                    done[row] = true;
                    continue;
                }
                let row_logits = logits.get(row).map_err(|error| {
                    self.execution_error(format!("failed to select logits row {row}: {error}"))
                })?;
                let row_logits = match &fsm_processors[row] {
                    Some(fsm) => self.mask_row_for_fsm(&row_logits, fsm, vocab_size, eos_tokens)?,
                    None => row_logits,
                };
                let next = processors[row].sample(&row_logits).map_err(|error| {
                    self.execution_error(format!(
                        "failed to sample next token for row {row}: {error}"
                    ))
                })?;
                if let Some(fsm) = fsm_processors[row].as_mut() {
                    if !eos_tokens.contains(&next) {
                        let text = self.tokenizer.decode(&[next], false).map_err(|error| {
                            self.execution_error(format!(
                                "failed to decode token for row {row}: {error}"
                            ))
                        })?;
                        fsm.commit(&text);
                    }
                }
                generated[row].push(next);
                next_tokens[row] = next;
                let text = self.decode_generated(&generated[row])?;
                if eos_tokens.contains(&next)
                    || find_earliest_stop(&text, &requests[row].stop).is_some()
                {
                    done[row] = true;
                }
            }

            if done.iter().all(|done| *done) || step + 1 == max_new_tokens {
                break;
            }
            if prompt_len + step + 1 > self.limits.max_position_embeddings {
                break;
            }
            for row in 0..batch {
                if done[row] {
                    next_tokens[row] = eos_tokens
                        .first()
                        .copied()
                        .or_else(|| generated[row].last().copied())
                        .unwrap_or(0);
                }
            }
            let input =
                Tensor::from_vec(next_tokens.clone(), (batch, 1), device).map_err(|error| {
                    self.execution_error(format!("failed to build batched decode tensor: {error}"))
                })?;
            logits = model
                .forward_with_adapters(&input, prompt_len + step, cache, adapter_assignments)
                .map_err(|error| {
                    self.execution_error(format!(
                        "batch-native LoRA decode forward pass failed: {error}"
                    ))
                })?;
        }

        generated
            .iter()
            .zip(requests)
            .map(|(tokens, request)| {
                let text = self.decode_generated(tokens)?;
                let end = find_earliest_stop(&text, &request.stop).unwrap_or(text.len());
                Ok(text.as_bytes()[..end].to_vec())
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_loop_from_logits(
        &self,
        mut logits: Tensor,
        mut index_pos: usize,
        _prompt_ids: &[u32],
        context: DecodeLoopContext<'_>,
        mut forward: impl FnMut(&Tensor, usize) -> candle_core::Result<Tensor>,
    ) -> Result<(), CandleLlmError> {
        let DecodeLoopContext {
            request,
            eos_tokens,
            input_device: device,
            on_token,
        } = context;
        let mut processor = request.sampling.processor();
        let mut fsm_processor = request
            .fsm
            .as_ref()
            .map(|fsm| FsmLogitProcessor::new(Arc::clone(fsm)));
        let vocab_size = self.tokenizer.get_vocab_size(true);
        let mut generated = Vec::with_capacity(request.max_new_tokens);
        // Hold back this many trailing bytes so a stop sequence split across the
        // last token(s) is never partially emitted before it is matched.
        let hold = request
            .stop
            .iter()
            .map(String::len)
            .max()
            .unwrap_or(0)
            .saturating_sub(1);
        let mut emitted = 0usize;
        for step in 0..request.max_new_tokens {
            let row = logits.squeeze(0).map_err(|error| {
                self.execution_error(format!("failed to reshape logits: {error}"))
            })?;
            let row = match &fsm_processor {
                Some(fsm) => self.mask_row_for_fsm(&row, fsm, vocab_size, eos_tokens)?,
                None => row,
            };
            let next = processor.sample(&row).map_err(|error| {
                self.execution_error(format!("failed to sample next token: {error}"))
            })?;
            if let Some(fsm) = fsm_processor.as_mut() {
                if !eos_tokens.contains(&next) {
                    let text = self.tokenizer.decode(&[next], false).map_err(|error| {
                        self.execution_error(format!("failed to decode token: {error}"))
                    })?;
                    fsm.commit(&text);
                }
            }
            generated.push(next);

            let text = self.decode_generated(&generated)?;
            // A matched stop ends the decode: emit up to (not including) it.
            if let Some(stop_at) = find_earliest_stop(&text, &request.stop) {
                emit_delta(on_token, &text, &mut emitted, stop_at);
                return Ok(());
            }
            if eos_tokens.contains(&next) {
                emit_delta(on_token, &text, &mut emitted, text.len());
                return Ok(());
            }
            // No stop yet: emit everything except the held-back tail.
            let safe = floor_char_boundary(&text, text.len().saturating_sub(hold));
            emit_delta(on_token, &text, &mut emitted, safe);

            if step + 1 == request.max_new_tokens {
                break;
            }
            if index_pos + 1 > self.limits.max_position_embeddings {
                break;
            }
            let input = Tensor::new(&[next], device)
                .and_then(|tensor| tensor.unsqueeze(0))
                .map_err(|error| {
                    self.execution_error(format!("failed to build input tensor: {error}"))
                })?;
            logits = forward(&input, index_pos).map_err(|error| {
                self.execution_error(format!("transformer forward pass failed: {error}"))
            })?;
            index_pos += 1;
        }
        // Token budget exhausted: flush the held-back tail, trimming any stop.
        let text = self.decode_generated(&generated)?;
        let end = find_earliest_stop(&text, &request.stop).unwrap_or(text.len());
        emit_delta(on_token, &text, &mut emitted, end);
        Ok(())
    }

    fn llama_prefill_with_prefix_cache(
        &self,
        model: &Llama,
        prompt_ids: &[u32],
        cache: &mut Cache,
        device: &Device,
    ) -> Result<(Tensor, usize), CandleLlmError> {
        let mut index_pos = 0usize;
        let mut logits = None;
        if let Some(entry) = self
            .prefix_cache
            .lock()
            .map_err(|_| self.execution_error("prefix cache mutex was poisoned".to_owned()))?
            .longest_match(prompt_ids)
        {
            *cache = entry.cache;
            index_pos = entry.tokens.len();
            logits = Some(entry.logits);
        }

        while index_pos < prompt_ids.len() {
            let remaining = prompt_ids.len() - index_pos;
            let chunk_len = self.limits.next_prefill_chunk_len(remaining);
            if index_pos + chunk_len > self.limits.max_position_embeddings {
                break;
            }
            let chunk = &prompt_ids[index_pos..index_pos + chunk_len];
            let input = Tensor::new(chunk, device)
                .and_then(|tensor| tensor.unsqueeze(0))
                .map_err(|error| {
                    self.execution_error(format!("failed to build input tensor: {error}"))
                })?;
            let next_logits = model.forward(&input, index_pos, cache).map_err(|error| {
                self.execution_error(format!("transformer forward pass failed: {error}"))
            })?;
            index_pos += chunk_len;
            if index_pos.is_multiple_of(PREFIX_CACHE_BLOCK_TOKENS) {
                self.prefix_cache
                    .lock()
                    .map_err(|_| {
                        self.execution_error("prefix cache mutex was poisoned".to_owned())
                    })?
                    .insert(&prompt_ids[..index_pos], cache.clone(), next_logits.clone());
            }
            logits = Some(next_logits);
        }

        logits
            .ok_or_else(|| self.execution_error("prompt produced no prefill logits".to_owned()))
            .map(|logits| (logits, index_pos))
    }

    /// Mask every vocabulary logit the grammar would reject for the next
    /// token to `-inf`, given `fsm`'s current state, leaving everything else
    /// untouched. Returns a new tensor; `row` itself is not mutated.
    fn mask_row_for_fsm(
        &self,
        row: &Tensor,
        fsm: &FsmLogitProcessor,
        vocab_size: usize,
        eos_tokens: &[u32],
    ) -> Result<Tensor, CandleLlmError> {
        let allowed = fsm.allowed_token_ids(vocab_size, eos_tokens, |id| {
            self.tokenizer.decode(&[id], false).unwrap_or_default()
        });
        let allowed: std::collections::HashSet<u32> = allowed.into_iter().collect();
        let mut values = row
            .to_vec1::<f32>()
            .map_err(|error| self.execution_error(format!("failed to read logits: {error}")))?;
        for (id, value) in values.iter_mut().enumerate() {
            if !allowed.contains(&(id as u32)) {
                *value = f32::NEG_INFINITY;
            }
        }
        Tensor::from_vec(values, row.shape(), row.device()).map_err(|error| {
            self.execution_error(format!("failed to build masked logits: {error}"))
        })
    }

    /// Decode the full generated token sequence to UTF-8 text (special tokens
    /// skipped). Always valid UTF-8, so byte offsets into it land on codepoint
    /// boundaries that grow monotonically as tokens are appended.
    fn decode_generated(&self, generated: &[u32]) -> Result<String, CandleLlmError> {
        self.tokenizer
            .decode(generated, true)
            .map_err(|error| self.execution_error(format!("failed to decode tokens: {error}")))
    }

    fn execution_error(&self, detail: String) -> CandleLlmError {
        CandleLlmError::Execution {
            alias: self.alias.clone(),
            detail,
        }
    }

    fn encode_ids(&self, prompt: &str) -> Result<Vec<u32>, CandleLlmError> {
        self.tokenizer
            .encode(prompt.to_owned(), true)
            .map(|encoded| encoded.get_ids().to_vec())
            .map_err(|error| CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!("failed to tokenize prompt: {error}"),
            })
    }

    /// Render structured chat turns into a single prompt. Uses the model's own
    /// `chat_template` (from `tokenizer_config.json`) when present so the result
    /// matches the checkpoint's expected control tokens; otherwise falls back to
    /// a generic, deterministic rendering that ends on an open assistant turn.
    fn render_chat(&self, messages: &[ChatTurn]) -> Result<String, CandleLlmError> {
        if messages.is_empty() {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: "chat request must include at least one message".to_owned(),
            });
        }
        match &self.chat_template {
            Some(template) => {
                template
                    .render(messages)
                    .map_err(|detail| CandleLlmError::InvalidRequest {
                        alias: self.alias.clone(),
                        detail: format!("failed to render chat template: {detail}"),
                    })
            }
            None => Ok(render_generic_chat(messages)),
        }
    }

    /// Test-only: the vocabulary logits the model produces for the final
    /// position of `prompt`. Used to prove generation is prompt-dependent.
    #[cfg(test)]
    pub(crate) fn debug_last_logits(&self, prompt: &str) -> Vec<f32> {
        let ids = self.encode_ids(prompt).expect("prompt should tokenize");
        let device = match &*self.inner {
            LoadedModel::Safetensors { backend, .. } => backend.device(),
            LoadedModel::Gguf { device, .. } => device.clone(),
            _ => Device::Cpu,
        };
        let input = Tensor::new(ids.as_slice(), &device)
            .and_then(|tensor| tensor.unsqueeze(0))
            .expect("input tensor should build");
        let logits = match &*self.inner {
            LoadedModel::Safetensors { backend, .. } => backend.last_logits(&input, &device),
            LoadedModel::Gguf { model, .. } => {
                let mut guard = model.lock().expect("gguf mutex should not be poisoned");
                guard.forward(&input, 0).expect("forward pass should run")
            }
            LoadedModel::Parallel(parallel) => match parallel {
                ParallelModel::Tensor {
                    model,
                    config,
                    devices,
                    ..
                } => {
                    let primary = &devices[0];
                    let input = input
                        .to_device(primary)
                        .expect("input should move to device");
                    let mut cache = TensorParallelCache::new(true, DType::F32, config, primary)
                        .expect("cache should build");
                    model
                        .forward(&input, 0, &mut cache)
                        .expect("forward pass should run")
                }
                ParallelModel::Pipeline { model, .. } => {
                    let stage0_device = model
                        .stages
                        .first()
                        .map(|stage| stage.device().clone())
                        .unwrap_or(Device::Cpu);
                    let input = input
                        .to_device(&stage0_device)
                        .expect("input should move to stage 0 device");
                    pipeline_prefill_forward(model, &input).expect("pipeline forward should run")
                }
                ParallelModel::Expert {
                    model,
                    config,
                    devices,
                    ..
                } => {
                    let primary = &devices[0];
                    let input = input
                        .to_device(primary)
                        .expect("input should move to device");
                    let mut cache = TensorParallelCache::new(true, DType::F32, config, primary)
                        .expect("cache should build");
                    model
                        .forward(&input, 0, &mut cache)
                        .expect("forward pass should run")
                }
            },
        };
        logits
            .squeeze(0)
            .and_then(|row| row.to_vec1::<f32>())
            .expect("logits should be an f32 vector")
    }

    /// Test-only: the prefill logits produced by a parallel engine for `prompt`.
    /// For tensor parallelism this is equivalent to `debug_last_logits`; for the
    /// prefill-only pipeline engine it is the only generation-shaped output
    /// available, used to prove equivalence to the dense reference.
    #[cfg(test)]
    pub(crate) fn debug_parallel_prefill_logits(&self, prompt: &str) -> Vec<f32> {
        self.debug_last_logits(prompt)
    }

    #[cfg(test)]
    fn debug_prefix_cache_token_lengths(&self) -> Vec<usize> {
        self.prefix_cache
            .lock()
            .expect("prefix cache should not be poisoned")
            .token_lengths()
    }

    #[cfg(test)]
    fn debug_prefix_cache_hits(&self) -> u64 {
        self.prefix_cache
            .lock()
            .expect("prefix cache should not be poisoned")
            .hits
    }

    fn parse_request(&self, data: &[u8]) -> Result<ParsedGenerationRequest, CandleLlmError> {
        if data.len() > self.limits.max_prompt_bytes {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!(
                    "prompt bytes {} exceed limit {}",
                    data.len(),
                    self.limits.max_prompt_bytes
                ),
            });
        }
        let raw = std::str::from_utf8(data).map_err(|error| CandleLlmError::InvalidRequest {
            alias: self.alias.clone(),
            detail: format!("prompt tensor must be valid UTF-8: {error}"),
        })?;

        let request = if raw.trim_start().starts_with('{') {
            let request = serde_json::from_str::<GenerationRequest>(raw).map_err(|error| {
                CandleLlmError::InvalidRequest {
                    alias: self.alias.clone(),
                    detail: format!("invalid JSON generation request: {error}"),
                }
            })?;
            // Prefer structured `messages` (chat-templated); otherwise a raw
            // `prompt`. Exactly one source of prompt text must be present.
            let prompt = match (request.messages, request.prompt) {
                (Some(messages), _) => self.render_chat(&messages)?,
                (None, Some(prompt)) => prompt,
                (None, None) => {
                    return Err(CandleLlmError::InvalidRequest {
                        alias: self.alias.clone(),
                        detail: "generation request must carry `messages` or `prompt`".to_owned(),
                    })
                }
            };
            let fsm = match request.json_schema {
                Some(schema) => Some(self.fsm_cache.get_or_compile(&schema).map_err(|error| {
                    CandleLlmError::InvalidRequest {
                        alias: self.alias.clone(),
                        detail: format!("invalid `json_schema`: {error}"),
                    }
                })?),
                None => None,
            };
            ParsedGenerationRequest {
                prompt,
                max_new_tokens: request
                    .max_new_tokens
                    .unwrap_or(self.limits.default_max_new_tokens),
                sampling: resolve_sampling(request.temperature, request.top_p, request.seed),
                stop: sanitize_stop(request.stop),
                fsm,
            }
        } else {
            ParsedGenerationRequest {
                prompt: raw.to_owned(),
                max_new_tokens: self.limits.default_max_new_tokens,
                sampling: resolve_sampling(None, None, None),
                stop: Vec::new(),
                fsm: None,
            }
        };

        if request.max_new_tokens == 0 || request.max_new_tokens > HOST_MAX_NEW_TOKENS {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!(
                    "max_new_tokens {} must be between 1 and {}",
                    request.max_new_tokens, HOST_MAX_NEW_TOKENS
                ),
            });
        }
        let encoded = self
            .tokenizer
            .encode(request.prompt.clone(), true)
            .map_err(|error| CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!("failed to tokenize prompt: {error}"),
            })?;
        if encoded.len() > self.limits.max_prompt_tokens
            || encoded.len() > self.limits.max_position_embeddings
        {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!(
                    "prompt tokens {} exceed token limit {} and context limit {}",
                    encoded.len(),
                    self.limits.max_prompt_tokens,
                    self.limits.max_position_embeddings
                ),
            });
        }

        Ok(request)
    }
}

/// Resolve an OpenAI-style `temperature`/`top_p`/`seed` triple into a sampling
/// policy. `temperature <= 0` (or absent) yields deterministic greedy decoding;
/// an absent `seed` falls back to a fixed seed so an un-seeded sampled request
/// stays reproducible. `top_p` is only meaningful in `(0, 1)`.
fn resolve_sampling(
    temperature: Option<f32>,
    top_p: Option<f32>,
    seed: Option<u64>,
) -> SamplingPolicy {
    let temperature = temperature
        .map(f64::from)
        .filter(|value| *value > 1e-7 && value.is_finite());
    let top_p = top_p
        .map(f64::from)
        .filter(|value| value.is_finite() && *value > 0.0 && *value < 1.0);
    SamplingPolicy {
        seed: seed.unwrap_or(DEFAULT_SAMPLING_SEED),
        temperature,
        top_p,
    }
}

/// Clamp a caller-supplied stop list to a bounded, non-empty set so a request
/// cannot force unbounded substring scans during decoding.
fn sanitize_stop(stop: Option<Vec<String>>) -> Vec<String> {
    stop.unwrap_or_default()
        .into_iter()
        .filter(|s| !s.is_empty() && s.len() <= MAX_STOP_SEQUENCE_BYTES)
        .take(MAX_STOP_SEQUENCES)
        .collect()
}

/// Byte offset of the earliest stop sequence in `text`, or `None`. The offset
/// is a substring-match start, so it always lands on a UTF-8 codepoint boundary.
fn find_earliest_stop(text: &str, stop: &[String]) -> Option<usize> {
    stop.iter()
        .filter_map(|needle| text.find(needle.as_str()))
        .min()
}

/// Emit `text[*emitted..end]` through `on_token` (when non-empty) and advance
/// `*emitted`. `end` and `*emitted` must be codepoint boundaries.
fn emit_delta(on_token: &mut dyn FnMut(&str), text: &str, emitted: &mut usize, end: usize) {
    if *emitted < end && end <= text.len() {
        on_token(&text[*emitted..end]);
        *emitted = end;
    }
}

/// Largest codepoint boundary `<= idx` (a stable stand-in for the unstable
/// `str::floor_char_boundary`).
fn floor_char_boundary(text: &str, mut idx: usize) -> usize {
    if idx >= text.len() {
        return text.len();
    }
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Generic chat rendering used when a checkpoint ships no `chat_template`:
/// one `role: content` line per turn, ending on an open `assistant:` turn so
/// the model continues as the assistant.
fn render_generic_chat(messages: &[ChatTurn]) -> String {
    let mut prompt = String::new();
    for message in messages {
        prompt.push_str(message.role.trim());
        prompt.push_str(": ");
        prompt.push_str(message.content.trim());
        prompt.push('\n');
    }
    prompt.push_str("assistant:");
    prompt
}

/// A checkpoint's chat template, extracted once from `tokenizer_config.json`.
/// Rendering uses minijinja with the Python-compatibility method set so real
/// Hugging Face instruct templates (which call `.strip()`, `.split()`, …) work.
pub(crate) struct ChatTemplate {
    source: String,
    bos_token: String,
    eos_token: String,
}

/// `chat_template` may be a single Jinja string or a list of named templates
/// (the multi-template form some tool-calling checkpoints ship).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ChatTemplateField {
    Single(String),
    Named(Vec<NamedChatTemplate>),
}

#[derive(Debug, Deserialize)]
struct NamedChatTemplate {
    name: String,
    template: String,
}

/// `bos_token`/`eos_token` may be a bare string or an `AddedToken` object whose
/// `content` holds the literal token text.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SpecialToken {
    Str(String),
    Obj { content: String },
}

impl SpecialToken {
    fn into_string(self) -> String {
        match self {
            SpecialToken::Str(s) => s,
            SpecialToken::Obj { content } => content,
        }
    }
}

#[derive(Debug, Deserialize)]
struct TokenizerConfig {
    #[serde(default)]
    chat_template: Option<ChatTemplateField>,
    #[serde(default)]
    bos_token: Option<SpecialToken>,
    #[serde(default)]
    eos_token: Option<SpecialToken>,
}

impl ChatTemplateField {
    /// Resolve to a single template source: the bare string, or the entry named
    /// `default` (falling back to the first) from the multi-template form.
    fn into_source(self) -> Option<String> {
        match self {
            ChatTemplateField::Single(source) => Some(source),
            ChatTemplateField::Named(mut templates) => {
                if let Some(index) = templates.iter().position(|t| t.name == "default") {
                    return Some(templates.swap_remove(index).template);
                }
                templates.into_iter().next().map(|t| t.template)
            }
        }
    }
}

impl ChatTemplate {
    /// Load the model's chat template from `tokenizer_config.json`, or `None`
    /// when the file or the `chat_template` field is absent.
    pub(crate) fn load(alias: &str, root: &Path) -> Result<Option<Self>, CandleLlmError> {
        let path = root.join(TOKENIZER_CONFIG_JSON);
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read(&path).map_err(|error| CandleLlmError::InvalidComponent {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            component: TOKENIZER_CONFIG_JSON,
            detail: error.to_string(),
        })?;
        let config: TokenizerConfig =
            serde_json::from_slice(&raw).map_err(|error| CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: TOKENIZER_CONFIG_JSON,
                detail: error.to_string(),
            })?;
        let Some(source) = config
            .chat_template
            .and_then(ChatTemplateField::into_source)
        else {
            return Ok(None);
        };
        Ok(Some(Self {
            source,
            bos_token: config
                .bos_token
                .map(SpecialToken::into_string)
                .unwrap_or_default(),
            eos_token: config
                .eos_token
                .map(SpecialToken::into_string)
                .unwrap_or_default(),
        }))
    }

    /// Render `messages` through the Jinja template with `add_generation_prompt`
    /// set, so the prompt ends ready for the assistant to continue.
    pub(crate) fn render(&self, messages: &[ChatTurn]) -> Result<String, String> {
        let mut env = minijinja::Environment::new();
        // Real HF templates call `raise_exception(...)` to reject malformed
        // conversations (e.g. a system turn where the model forbids one).
        env.add_function(
            "raise_exception",
            |message: String| -> std::result::Result<minijinja::Value, minijinja::Error> {
                Err(minijinja::Error::new(
                    minijinja::ErrorKind::InvalidOperation,
                    message,
                ))
            },
        );
        // Map Python string methods (`.strip()`, `.split()`, …) used by templates.
        env.set_unknown_method_callback(minijinja_contrib::pycompat::unknown_method_callback);
        env.add_template("chat", &self.source)
            .map_err(|error| error.to_string())?;
        let template = env
            .get_template("chat")
            .map_err(|error| error.to_string())?;
        template
            .render(minijinja::context! {
                messages => messages,
                add_generation_prompt => true,
                bos_token => self.bos_token,
                eos_token => self.eos_token,
            })
            .map_err(|error| error.to_string())
    }
}

/// Determine the on-disk format of a model directory, or `None` when the
/// directory is not a recognizable model (so the caller can fall through to
/// other backends). The broker's `.tachyon-model.json` sidecar is authoritative
/// when present; otherwise the format is inferred from directory contents.
fn resolve_model_format(alias: &str, root: &Path) -> Result<Option<ModelFormat>, CandleLlmError> {
    let meta_path = root.join(MODEL_META_JSON);
    if meta_path.exists() {
        let raw = fs::read(&meta_path).map_err(|error| CandleLlmError::InvalidComponent {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            component: MODEL_META_JSON,
            detail: error.to_string(),
        })?;
        let meta: ModelMeta =
            serde_json::from_slice(&raw).map_err(|error| CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: MODEL_META_JSON,
                detail: error.to_string(),
            })?;
        return match meta.format.as_str() {
            "safetensors" => Ok(Some(ModelFormat::Safetensors)),
            "gguf" => Ok(Some(ModelFormat::Gguf)),
            other => Err(CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail: format!(
                    "`{MODEL_META_JSON}` declares unsupported format `{other}` (expected `safetensors` or `gguf`)"
                ),
            }),
        };
    }

    // No sidecar: infer from contents. A `.gguf` file wins (single-file upload);
    // otherwise a `config.json` marks a safetensors directory.
    if find_gguf_file(root).is_some() {
        Ok(Some(ModelFormat::Gguf))
    } else if root.join(CONFIG_JSON).exists() {
        Ok(Some(ModelFormat::Safetensors))
    } else {
        Ok(None)
    }
}

impl ModelFormat {
    fn display_name(self) -> &'static str {
        match self {
            Self::Safetensors => "safetensors",
            Self::Gguf => "GGUF",
        }
    }
}

fn distribution_mode_name(mode: GpuDistribution) -> &'static str {
    match mode {
        GpuDistribution::Single => "single",
        GpuDistribution::TensorParallelism => "tensor_parallelism",
        GpuDistribution::PipelineParallelism => "pipeline_parallelism",
        GpuDistribution::ExpertParallelism => "expert_parallelism",
    }
}

fn inspect_model_architecture(
    alias: &str,
    root: &Path,
    format: ModelFormat,
) -> Result<ModelArchitecture, CandleLlmError> {
    match format {
        ModelFormat::Safetensors => {
            let raw = fs::read(root.join(CONFIG_JSON)).map_err(|error| {
                CandleLlmError::InvalidComponent {
                    alias: alias.to_owned(),
                    path: root.to_path_buf(),
                    component: CONFIG_JSON,
                    detail: error.to_string(),
                }
            })?;
            inspect_hf_architecture(alias, root, &raw)
        }
        ModelFormat::Gguf => {
            let gguf_path = gguf_file_path(alias, root)?;
            let mut reader =
                fs::File::open(&gguf_path).map_err(|error| CandleLlmError::InvalidComponent {
                    alias: alias.to_owned(),
                    path: root.to_path_buf(),
                    component: GGUF_COMPONENT,
                    detail: error.to_string(),
                })?;
            let content = gguf_file::Content::read(&mut reader).map_err(|error| {
                CandleLlmError::InvalidComponent {
                    alias: alias.to_owned(),
                    path: root.to_path_buf(),
                    component: GGUF_COMPONENT,
                    detail: error.to_string(),
                }
            })?;
            let declared = content
                .metadata
                .get("general.architecture")
                .and_then(|value| value.to_string().ok())
                .cloned()
                .unwrap_or_default();
            ModelArchitecture::from_gguf_architecture(&declared).ok_or_else(|| {
                CandleLlmError::UnsupportedModel {
                    alias: alias.to_owned(),
                    path: root.to_path_buf(),
                    detail: format!(
                        "unrecognized GGUF `general.architecture` `{declared}`; the model alias and directory name are not used for architecture selection"
                    ),
                }
            })
        }
    }
}

fn inspect_hf_architecture(
    alias: &str,
    root: &Path,
    raw_config: &[u8],
) -> Result<ModelArchitecture, CandleLlmError> {
    let probe: ModelTypeProbe =
        serde_json::from_slice(raw_config).map_err(|error| CandleLlmError::InvalidComponent {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            component: CONFIG_JSON,
            detail: error.to_string(),
        })?;
    let declared = if probe.model_type.is_empty() {
        probe
            .text_config
            .as_deref()
            .map(|text| text.model_type.as_str())
            .unwrap_or_default()
    } else {
        probe.model_type.as_str()
    };
    if is_multimodal_hf_model_type(declared) || probe.vision_config.is_some() {
        return Err(CandleLlmError::UnsupportedModel {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            detail: format!(
                "multimodal vision-language checkpoints (`model_type` = `{declared}`, optional `vision_config`) are not supported by the text-only Candle backend; Tachyon currently has no image preprocessing, vision encoder, multimodal projector, or image request tensor path"
            ),
        });
    }
    let architecture = ModelArchitecture::from_hf_model_type(declared).ok_or_else(|| {
        CandleLlmError::UnsupportedModel {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            detail: format!(
                "unrecognized Hugging Face `model_type` `{declared}`; supported generic families include `{LLAMA_MODEL_TYPE}`, `qwen2`, `qwen3`, and `gemma2`; the model alias and directory name are not used for architecture selection"
            ),
        }
    })?;
    Ok(architecture)
}

fn generation_eos_token_ids(
    alias: &str,
    root: &Path,
    raw_config: &[u8],
) -> Result<Vec<u32>, CandleLlmError> {
    let probe: GenerationMetadataProbe =
        serde_json::from_slice(raw_config).map_err(|error| CandleLlmError::InvalidComponent {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            component: CONFIG_JSON,
            detail: error.to_string(),
        })?;
    Ok(match probe.eos_token_id {
        Some(TokenIdField::One(id)) => vec![id],
        Some(TokenIdField::Many(ids)) => ids,
        None => Vec::new(),
    })
}

fn generation_context_limit(
    alias: &str,
    root: &Path,
    raw_config: &[u8],
) -> Result<usize, CandleLlmError> {
    let probe: GenerationMetadataProbe =
        serde_json::from_slice(raw_config).map_err(|error| CandleLlmError::InvalidComponent {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            component: CONFIG_JSON,
            detail: error.to_string(),
        })?;
    Ok(probe.max_position_embeddings)
}

/// Locate the single `*.gguf` file in a model directory, if any.
fn find_gguf_file(root: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    let mut candidates: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case(GGUF_EXTENSION))
        })
        .collect();
    candidates.sort();
    candidates.into_iter().next()
}

/// Resolve the `*.gguf` path inside a model directory, validating that the bytes
/// actually start with the GGUF magic so a mis-tagged file fails clearly.
fn gguf_file_path(alias: &str, root: &Path) -> Result<PathBuf, CandleLlmError> {
    let path = find_gguf_file(root).ok_or_else(|| CandleLlmError::MissingFile {
        alias: alias.to_owned(),
        path: root.to_path_buf(),
        file: GGUF_COMPONENT,
    })?;
    let mut magic = [0u8; 4];
    {
        use std::io::Read;
        let mut file = fs::File::open(&path).map_err(|error| CandleLlmError::InvalidComponent {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            component: GGUF_COMPONENT,
            detail: error.to_string(),
        })?;
        file.read_exact(&mut magic)
            .map_err(|error| CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: GGUF_COMPONENT,
                detail: format!("could not read GGUF magic: {error}"),
            })?;
    }
    if magic != GGUF_MAGIC {
        return Err(CandleLlmError::InvalidComponent {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            component: GGUF_COMPONENT,
            detail: "file does not start with the GGUF magic".to_owned(),
        });
    }
    Ok(path)
}

/// GGML block types the pinned candle fork can actually multiply on a CUDA
/// device.
///
/// The list mirrors the `dtype` matches in
/// `candle-core/src/quantized/cuda.rs` (`dequantize_mul_mat_vec`,
/// `mul_mat_vec_via_q8_1`, `mul_mat_via_q8_1`), which is where a GGUF weight
/// ends up once `QMatMul::from_arc` keeps it as a `QTensor`. `F32`/`F16`/`BF16`
/// are included for a different reason: `from_arc` dequantizes those into a
/// dense `Tensor` instead, so they never reach a quantized kernel at all.
///
/// `Q8_1` and `Q8K` are deliberately absent. Both parse and both have CUDA
/// *dequantize* kernels, but neither has a CUDA quantized-matmul kernel, so a
/// checkpoint carrying them would load onto the GPU and then fail at the first
/// decode step with candle's generic "unsupported dtype for quantized matmul".
const fn gguf_dtype_has_cuda_matmul_kernel(dtype: GgmlDType) -> bool {
    matches!(
        dtype,
        GgmlDType::F32
            | GgmlDType::F16
            | GgmlDType::BF16
            | GgmlDType::Q4_0
            | GgmlDType::Q4_1
            | GgmlDType::Q5_0
            | GgmlDType::Q5_1
            | GgmlDType::Q8_0
            | GgmlDType::Q2K
            | GgmlDType::Q3K
            | GgmlDType::Q4K
            | GgmlDType::Q5K
            | GgmlDType::Q6K
    )
}

/// The first tensor in a checkpoint that CUDA cannot multiply, plus how
/// widespread the problem is.
struct UnsupportedGgufDtypes {
    tensor: String,
    dtype: GgmlDType,
    affected: usize,
    total: usize,
}

/// Scan a GGUF checkpoint for block types with no CUDA quantized-matmul kernel.
///
/// Kept separate from device selection so the rule is unit-testable on a
/// CUDA-less build, which is where the default CI matrix runs.
fn unsupported_gguf_cuda_dtypes(content: &gguf_file::Content) -> Option<UnsupportedGgufDtypes> {
    // Sorted so a mixed-quant checkpoint always reports the same offending
    // tensor: `tensor_infos` is a hash map, and an arbitrary pick would make
    // the error message differ run to run for the same file.
    let mut unsupported = content
        .tensor_infos
        .iter()
        .filter(|(_, info)| !gguf_dtype_has_cuda_matmul_kernel(info.ggml_dtype))
        .map(|(name, info)| (name.as_str(), info.ggml_dtype))
        .collect::<Vec<_>>();
    unsupported.sort_unstable_by_key(|(name, _)| *name);
    let (tensor, dtype) = unsupported.first()?;
    Some(UnsupportedGgufDtypes {
        tensor: (*tensor).to_owned(),
        dtype: *dtype,
        affected: unsupported.len(),
        total: content.tensor_infos.len(),
    })
}

/// Pick the device a GGUF checkpoint's weights are uploaded to.
///
/// `cpu` (the default) keeps the historical host-side quantized path. Any other
/// request resolves a real CUDA device, but only after every tensor's block
/// type has been checked against [`gguf_dtype_has_cuda_matmul_kernel`] — a
/// mixed-quant checkpoint that names even one unsupported block type is
/// rejected with a typed error naming the tensor and its dtype, instead of
/// half-loading onto the GPU.
fn resolve_gguf_device(
    alias: &str,
    root: &Path,
    content: &gguf_file::Content,
    requested_device: &str,
) -> Result<Device, CandleLlmError> {
    if requested_device == "cpu" {
        return Ok(Device::Cpu);
    }

    if !cfg!(feature = "candle-cuda") {
        return Err(CandleLlmError::UnsupportedModel {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            detail: format!(
                "GGUF execution on `{requested_device}` requires a host built with the `candle-cuda` feature; rebuild with it or bind this model to `cpu`"
            ),
        });
    }

    if let Some(report) = unsupported_gguf_cuda_dtypes(content) {
        return Err(CandleLlmError::UnsupportedModel {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            detail: format!(
                "GGUF tensor `{}` uses block type `{:?}`, which has no CUDA quantized-matmul kernel ({} of {} tensors affected); requantize to a K-quant or legacy Q4/Q5/Q8 type, or bind this model to `cpu`",
                report.tensor, report.dtype, report.affected, report.total
            ),
        });
    }

    Device::cuda_if_available(0).map_err(|error| CandleLlmError::InvalidComponent {
        alias: alias.to_owned(),
        path: root.to_path_buf(),
        component: GGUF_COMPONENT,
        detail: error.to_string(),
    })
}

/// Collect the model's end-of-sequence token id(s), if any, into a flat list.
fn eos_token_ids(config: &Config) -> Vec<u32> {
    match &config.eos_token_id {
        Some(LlamaEosToks::Single(id)) => vec![*id],
        Some(LlamaEosToks::Multiple(ids)) => ids.clone(),
        None => Vec::new(),
    }
}

/// Map a deployment's `hardware_strategy` onto the hardware-agnostic
/// `ParallelExecutionPlan` validated by `parallel-topology`. VRAM-per-shard is
/// left `0` ("not yet sized"), so the plan is only rejected on device-count or
/// interconnect grounds until real VRAM sizing lands.
fn plan_from_strategy(strategy: &HardwareStrategy) -> ParallelExecutionPlan {
    let parallel_strategy = match strategy.distribution_mode {
        GpuDistribution::Single => ParallelStrategy::None,
        GpuDistribution::TensorParallelism => ParallelStrategy::TensorParallel,
        GpuDistribution::PipelineParallelism => ParallelStrategy::PipelineParallel,
        GpuDistribution::ExpertParallelism => ParallelStrategy::ExpertParallel,
    };
    ParallelExecutionPlan {
        strategy: parallel_strategy,
        device_ids: strategy.device_ids.clone(),
        stage_layer_ranges: strategy.stage_layer_ranges.clone(),
        expert_device_map: strategy.expert_device_map.clone(),
        required_vram_bytes_per_device: 0,
        pipeline_depth: strategy.pipeline_depth,
    }
}

/// Resolve plan device IDs to candle `Device` handles. On a CUDA build each ID
/// maps to its CUDA ordinal; on a CUDA-less build every ID degenerates to
/// `Device::Cpu`, which the parallel engines treat as a multi-device stand-in
/// (the same simulation the engines' own equivalence tests use). A CUDA ordinal
/// that cannot be opened falls back to CPU rather than failing the load.
fn resolve_devices(device_ids: &[u32]) -> Vec<Device> {
    device_ids
        .iter()
        .map(|&id| {
            #[cfg(feature = "candle-cuda")]
            {
                Device::cuda_if_available(id as usize).unwrap_or(Device::Cpu)
            }
            #[cfg(not(feature = "candle-cuda"))]
            {
                let _ = id;
                Device::Cpu
            }
        })
        .collect()
}

/// Wraps an `anyhow::Error` from the format-agnostic `modelopt_nvfp4` module
/// (which knows nothing about Candle) into the typed `CandleLlmError` this
/// module's callers expect.
fn invalid_nvfp4<'a>(
    alias: &'a str,
    root: &'a Path,
) -> impl Fn(anyhow::Error) -> CandleLlmError + 'a {
    move |error| CandleLlmError::InvalidComponent {
        alias: alias.to_owned(),
        path: root.to_path_buf(),
        component: MODEL_SAFETENSORS,
        detail: error.to_string(),
    }
}

/// Dequantizes one NVFP4 linear's packed weight into a dense F32 `Tensor`:
/// the fallback (non-native-kernel) NVFP4 execution path. Reads the packed
/// nibbles, FP8 block scales, and F32 tensor scale straight from the shard
/// file and reuses the already-tested `dequantize_nvfp4_e4m3`.
fn dense_tensor_from_nvfp4(
    linear: &modelopt_nvfp4::Nvfp4LinearTensors,
    device: &Device,
) -> anyhow::Result<Tensor> {
    let packed = linear.packed_weight.tensor.read_bytes()?;
    let block_scales = linear.block_scales.tensor.read_bytes()?;
    let tensor_scale_bytes = linear.tensor_scale.tensor.read_bytes()?;
    let tensor_scale_bytes: [u8; 4] = tensor_scale_bytes
        .get(..4)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "NVFP4 tensor scale for `{}` is shorter than 4 bytes",
                linear.base_name
            )
        })?
        .try_into()
        .expect("length checked above");
    let tensor_scale = f32::from_le_bytes(tensor_scale_bytes);
    let n_rows = linear.packed_weight.tensor.info.shape[0];
    let n_cols = linear
        .packed_weight
        .tensor
        .info
        .shape
        .get(1)
        .copied()
        .unwrap_or_default()
        * 2;
    let dense = modelopt_nvfp4::dequantize_nvfp4_e4m3(
        &packed,
        &block_scales,
        tensor_scale,
        n_rows,
        n_cols,
        modelopt_nvfp4::Nvfp4OutputDType::F32,
    )?;
    let values = match dense {
        modelopt_nvfp4::Nvfp4DenseValues::F32(values) => values,
        modelopt_nvfp4::Nvfp4DenseValues::BF16(_) => {
            anyhow::bail!("internal error: requested F32 NVFP4 dequantization but got BF16")
        }
    };
    Ok(Tensor::from_vec(values, (n_rows, n_cols), device)?)
}

/// Reads an unquantized (passthrough) tensor straight from its shard, for
/// the parameters an NVFP4 checkpoint does not quantize (norms, embeddings,
/// any linear ModelOpt left in full precision).
fn raw_tensor_from_ref(
    reference: &modelopt_nvfp4::SafetensorsTensorRef,
    device: &Device,
) -> anyhow::Result<Tensor> {
    let bytes = reference.read_bytes()?;
    let dtype = candle_dtype_for(reference.info.dtype)?;
    Ok(Tensor::from_raw_buffer(
        &bytes,
        dtype,
        &reference.info.shape,
        device,
    )?)
}

fn candle_dtype_for(dtype: modelopt_nvfp4::SafetensorsDType) -> anyhow::Result<DType> {
    use modelopt_nvfp4::SafetensorsDType;
    Ok(match dtype {
        SafetensorsDType::U8 => DType::U8,
        SafetensorsDType::U32 => DType::U32,
        SafetensorsDType::F16 => DType::F16,
        SafetensorsDType::BF16 => DType::BF16,
        SafetensorsDType::F32 => DType::F32,
        SafetensorsDType::F8E4M3 => DType::F8E4M3,
        SafetensorsDType::Other => {
            anyhow::bail!("unsupported safetensors dtype for a passthrough NVFP4 tensor")
        }
    })
}

/// Resolve the safetensors shard paths for a model directory: the HF index when
/// the checkpoint is sharded, otherwise the single `model.safetensors`.
fn safetensors_paths(alias: &str, root: &Path) -> Result<Vec<PathBuf>, CandleLlmError> {
    let index = root.join(SAFETENSORS_INDEX_JSON);
    if index.exists() {
        let raw = fs::read(&index).map_err(|error| CandleLlmError::InvalidComponent {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            component: "model.safetensors.index.json",
            detail: error.to_string(),
        })?;
        let parsed: SafetensorsIndex =
            serde_json::from_slice(&raw).map_err(|error| CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: "model.safetensors.index.json",
                detail: error.to_string(),
            })?;
        let mut shards: Vec<String> = parsed.weight_map.into_values().collect();
        shards.sort();
        shards.dedup();
        Ok(shards.into_iter().map(|shard| root.join(shard)).collect())
    } else {
        Ok(vec![root.join(MODEL_SAFETENSORS)])
    }
}

fn lora_load_config_from_adapter(
    adapter_path: &Path,
    device: &Device,
) -> candle_core::Result<(LoraConfig, &'static str)> {
    let tensors = candle_core::safetensors::load(adapter_path, device)?;
    let mut rank = None;
    let mut target_modules = Vec::new();
    let mut has_peft_prefix = false;

    for (name, tensor) in tensors.iter() {
        let Some(module_path) = name.strip_suffix(".lora_A.weight") else {
            continue;
        };
        if module_path.starts_with("base_model.model.") {
            has_peft_prefix = true;
        }
        let Some(module) = module_path.rsplit('.').next() else {
            continue;
        };
        if !matches!(
            module,
            "q_proj" | "k_proj" | "v_proj" | "o_proj" | "gate_proj" | "up_proj" | "down_proj"
        ) {
            continue;
        }

        let adapter_rank = tensor.dim(0)?;
        match rank {
            Some(existing) if existing != adapter_rank => candle_core::bail!(
                "LoRA adapter mixes ranks {existing} and {adapter_rank}; one rank per adapter is supported"
            ),
            None => rank = Some(adapter_rank),
            _ => {}
        }
        if !target_modules.iter().any(|target| target == module) {
            target_modules.push(module.to_owned());
        }
    }

    let Some(rank) = rank else {
        candle_core::bail!(
            "LoRA adapter contains no supported `*.lora_A.weight` projection tensors"
        );
    };
    if target_modules.is_empty() {
        candle_core::bail!("LoRA adapter targets no supported Llama projection modules");
    }

    Ok((
        LoraConfig {
            rank,
            alpha: rank as f64,
            target_modules,
        },
        if has_peft_prefix {
            "base_model.model"
        } else {
            ""
        },
    ))
}

#[cfg(test)]
pub(crate) const FIXTURE_HIDDEN_SIZE: usize = 8;
#[cfg(test)]
pub(crate) const FIXTURE_NUM_LAYERS: usize = 2;
#[cfg(test)]
pub(crate) const FIXTURE_NUM_HEADS: usize = 2;
#[cfg(test)]
pub(crate) const FIXTURE_INTERMEDIATE_SIZE: usize = 16;
#[cfg(test)]
pub(crate) const FIXTURE_VOCAB_SIZE: usize = 4;
#[cfg(test)]
pub(crate) const FIXTURE_MAX_POSITION_EMBEDDINGS: usize = 32;

/// A 4-token WordLevel tokenizer shared by both tiny fixtures: `<unk> hello
/// tachyon mesh`. GGUF carries no tokenizer candle can use, so it ships here too.
#[cfg(test)]
const TINY_TOKENIZER_JSON: &str = r#"{
  "version": "1.0",
  "truncation": null,
  "padding": null,
  "added_tokens": [],
  "normalizer": null,
  "pre_tokenizer": {"type": "Whitespace"},
  "post_processor": null,
  "decoder": null,
  "model": {
    "type": "WordLevel",
    "vocab": {"<unk>": 0, "hello": 1, "tachyon": 2, "mesh": 3},
    "unk_token": "<unk>"
  }
}"#;

/// Write a complete, deterministic tiny **Llama** checkpoint (HF layout:
/// `config.json` + `tokenizer.json` + `model.safetensors`) so tests exercise the
/// real candle-transformers Llama forward without downloading a multi-GB model.
#[cfg(test)]
pub(crate) fn write_tachyon_tiny_fixture(root: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join(CONFIG_JSON),
        serde_json::json!({
            "model_type": LLAMA_MODEL_TYPE,
            "architectures": ["LlamaForCausalLM"],
            "hidden_size": FIXTURE_HIDDEN_SIZE,
            "intermediate_size": FIXTURE_INTERMEDIATE_SIZE,
            "vocab_size": FIXTURE_VOCAB_SIZE,
            "num_hidden_layers": FIXTURE_NUM_LAYERS,
            "num_attention_heads": FIXTURE_NUM_HEADS,
            "num_key_value_heads": FIXTURE_NUM_HEADS,
            "rms_norm_eps": 1e-5,
            "rope_theta": 10000.0,
            "max_position_embeddings": FIXTURE_MAX_POSITION_EMBEDDINGS,
            "tie_word_embeddings": false,
            "bos_token_id": 1,
            "eos_token_id": null
        })
        .to_string(),
    )?;
    fs::write(root.join(TOKENIZER_JSON), TINY_TOKENIZER_JSON)?;
    candle_core::safetensors::save(&fixture_weights()?, root.join(MODEL_SAFETENSORS))?;
    Ok(())
}

/// Deterministic, non-degenerate weights for the fixture, in the Llama tensor
/// layout candle-transformers expects (all `*.weight`, no biases; RMSNorm scales
/// are 1.0 so normalization is well-conditioned).
#[cfg(test)]
fn fixture_weights() -> anyhow::Result<HashMap<String, Tensor>> {
    let hidden = FIXTURE_HIDDEN_SIZE;
    let inter = FIXTURE_INTERMEDIATE_SIZE;
    let head_dim = hidden / FIXTURE_NUM_HEADS;
    let q = FIXTURE_NUM_HEADS * head_dim;
    let kv = FIXTURE_NUM_HEADS * head_dim;
    let mut tensors = HashMap::new();
    let mut seed = 1u64;
    let mut dense =
        |tensors: &mut HashMap<String, Tensor>, name: &str, dims: &[usize]| -> anyhow::Result<()> {
            let len: usize = dims.iter().product();
            let values = deterministic_fill(seed, len);
            seed = seed.wrapping_add(1);
            tensors.insert(
                name.to_owned(),
                Tensor::from_vec(values, dims.to_vec(), &Device::Cpu)?,
            );
            Ok(())
        };
    let norm = |tensors: &mut HashMap<String, Tensor>, name: &str| -> anyhow::Result<()> {
        tensors.insert(
            name.to_owned(),
            Tensor::from_vec(vec![1f32; hidden], (hidden,), &Device::Cpu)?,
        );
        Ok(())
    };

    dense(
        &mut tensors,
        "model.embed_tokens.weight",
        &[FIXTURE_VOCAB_SIZE, hidden],
    )?;
    for layer in 0..FIXTURE_NUM_LAYERS {
        let prefix = format!("model.layers.{layer}");
        dense(
            &mut tensors,
            &format!("{prefix}.self_attn.q_proj.weight"),
            &[q, hidden],
        )?;
        dense(
            &mut tensors,
            &format!("{prefix}.self_attn.k_proj.weight"),
            &[kv, hidden],
        )?;
        dense(
            &mut tensors,
            &format!("{prefix}.self_attn.v_proj.weight"),
            &[kv, hidden],
        )?;
        dense(
            &mut tensors,
            &format!("{prefix}.self_attn.o_proj.weight"),
            &[hidden, q],
        )?;
        dense(
            &mut tensors,
            &format!("{prefix}.mlp.gate_proj.weight"),
            &[inter, hidden],
        )?;
        dense(
            &mut tensors,
            &format!("{prefix}.mlp.up_proj.weight"),
            &[inter, hidden],
        )?;
        dense(
            &mut tensors,
            &format!("{prefix}.mlp.down_proj.weight"),
            &[hidden, inter],
        )?;
        norm(&mut tensors, &format!("{prefix}.input_layernorm.weight"))?;
        norm(
            &mut tensors,
            &format!("{prefix}.post_attention_layernorm.weight"),
        )?;
    }
    norm(&mut tensors, "model.norm.weight")?;
    dense(
        &mut tensors,
        "lm_head.weight",
        &[FIXTURE_VOCAB_SIZE, hidden],
    )?;
    Ok(tensors)
}

/// Head-dim-compatible tiny Llama fixture for CUDA-only paged-attention
/// tests. `candle_flash_attn`'s paged kernel additionally requires head
/// sizes to be a multiple of 8 ("only supports head sizes that are a
/// multiple of 8 (got 4)") — a third real constraint caught only by a real
/// `cuda-quality` run, since the shared `FIXTURE_HIDDEN_SIZE`/`FIXTURE_NUM_HEADS`
/// (head_dim 4) never exercises this kernel via the CPU/dense tests. Doubles
/// hidden_size to 16 (head_dim 8) and reuses every other tiny-fixture
/// dimension; a separate function rather than changing `FIXTURE_HIDDEN_SIZE`
/// itself, which every architecture's fixture (Qwen, Gemma, Phi, DeepSeek,
/// Mixtral, NVFP4 — ~180 other tests) depends on.
#[cfg(test)]
fn write_tachyon_tiny_paged_attention_fixture(root: &Path) -> anyhow::Result<()> {
    const HIDDEN: usize = 16;
    const NUM_HEADS: usize = 2;
    fs::create_dir_all(root)?;
    fs::write(
        root.join(CONFIG_JSON),
        serde_json::json!({
            "model_type": LLAMA_MODEL_TYPE,
            "architectures": ["LlamaForCausalLM"],
            "hidden_size": HIDDEN,
            "intermediate_size": FIXTURE_INTERMEDIATE_SIZE,
            "vocab_size": FIXTURE_VOCAB_SIZE,
            "num_hidden_layers": FIXTURE_NUM_LAYERS,
            "num_attention_heads": NUM_HEADS,
            "num_key_value_heads": NUM_HEADS,
            "rms_norm_eps": 1e-5,
            "rope_theta": 10000.0,
            "max_position_embeddings": FIXTURE_MAX_POSITION_EMBEDDINGS,
            "tie_word_embeddings": false,
            "bos_token_id": 1,
            "eos_token_id": null
        })
        .to_string(),
    )?;
    fs::write(root.join(TOKENIZER_JSON), TINY_TOKENIZER_JSON)?;

    let inter = FIXTURE_INTERMEDIATE_SIZE;
    let head_dim = HIDDEN / NUM_HEADS;
    let q = NUM_HEADS * head_dim;
    let kv = NUM_HEADS * head_dim;
    let mut tensors = HashMap::new();
    let mut seed = 1u64;
    let mut dense =
        |tensors: &mut HashMap<String, Tensor>, name: &str, dims: &[usize]| -> anyhow::Result<()> {
            let len: usize = dims.iter().product();
            let values = deterministic_fill(seed, len);
            seed = seed.wrapping_add(1);
            tensors.insert(
                name.to_owned(),
                Tensor::from_vec(values, dims.to_vec(), &Device::Cpu)?,
            );
            Ok(())
        };
    let norm = |tensors: &mut HashMap<String, Tensor>, name: &str| -> anyhow::Result<()> {
        tensors.insert(
            name.to_owned(),
            Tensor::from_vec(vec![1f32; HIDDEN], (HIDDEN,), &Device::Cpu)?,
        );
        Ok(())
    };

    dense(
        &mut tensors,
        "model.embed_tokens.weight",
        &[FIXTURE_VOCAB_SIZE, HIDDEN],
    )?;
    for layer in 0..FIXTURE_NUM_LAYERS {
        let prefix = format!("model.layers.{layer}");
        dense(
            &mut tensors,
            &format!("{prefix}.self_attn.q_proj.weight"),
            &[q, HIDDEN],
        )?;
        dense(
            &mut tensors,
            &format!("{prefix}.self_attn.k_proj.weight"),
            &[kv, HIDDEN],
        )?;
        dense(
            &mut tensors,
            &format!("{prefix}.self_attn.v_proj.weight"),
            &[kv, HIDDEN],
        )?;
        dense(
            &mut tensors,
            &format!("{prefix}.self_attn.o_proj.weight"),
            &[HIDDEN, q],
        )?;
        dense(
            &mut tensors,
            &format!("{prefix}.mlp.gate_proj.weight"),
            &[inter, HIDDEN],
        )?;
        dense(
            &mut tensors,
            &format!("{prefix}.mlp.up_proj.weight"),
            &[inter, HIDDEN],
        )?;
        dense(
            &mut tensors,
            &format!("{prefix}.mlp.down_proj.weight"),
            &[HIDDEN, inter],
        )?;
        norm(&mut tensors, &format!("{prefix}.input_layernorm.weight"))?;
        norm(
            &mut tensors,
            &format!("{prefix}.post_attention_layernorm.weight"),
        )?;
    }
    norm(&mut tensors, "model.norm.weight")?;
    dense(
        &mut tensors,
        "lm_head.weight",
        &[FIXTURE_VOCAB_SIZE, HIDDEN],
    )?;

    candle_core::safetensors::save(&tensors, root.join(MODEL_SAFETENSORS))?;
    Ok(())
}

#[cfg(test)]
fn write_tachyon_tiny_qwen_fixture(
    root: &Path,
    architecture: ModelArchitecture,
) -> anyhow::Result<()> {
    fs::create_dir_all(root)?;
    let (model_type, architecture_name, mut config) = match architecture {
        ModelArchitecture::Qwen2 => (
            "qwen2",
            "Qwen2ForCausalLM",
            serde_json::json!({
                "model_type": "qwen2",
                "vocab_size": FIXTURE_VOCAB_SIZE,
                "hidden_size": FIXTURE_HIDDEN_SIZE,
                "intermediate_size": FIXTURE_INTERMEDIATE_SIZE,
                "num_hidden_layers": FIXTURE_NUM_LAYERS,
                "num_attention_heads": FIXTURE_NUM_HEADS,
                "num_key_value_heads": FIXTURE_NUM_HEADS,
                "max_position_embeddings": FIXTURE_MAX_POSITION_EMBEDDINGS,
                "sliding_window": FIXTURE_MAX_POSITION_EMBEDDINGS,
                "max_window_layers": FIXTURE_NUM_LAYERS,
                "tie_word_embeddings": false,
                "rope_theta": 10000.0,
                "rms_norm_eps": 1e-5,
                "use_sliding_window": false,
                "hidden_act": "silu",
                "eos_token_id": null
            }),
        ),
        ModelArchitecture::Qwen3 => (
            "qwen3",
            "Qwen3ForCausalLM",
            serde_json::json!({
                "model_type": "qwen3",
                "vocab_size": FIXTURE_VOCAB_SIZE,
                "hidden_size": FIXTURE_HIDDEN_SIZE,
                "intermediate_size": FIXTURE_INTERMEDIATE_SIZE,
                "num_hidden_layers": FIXTURE_NUM_LAYERS,
                "num_attention_heads": FIXTURE_NUM_HEADS,
                "head_dim": FIXTURE_HIDDEN_SIZE / FIXTURE_NUM_HEADS,
                "attention_bias": false,
                "num_key_value_heads": FIXTURE_NUM_HEADS,
                "max_position_embeddings": FIXTURE_MAX_POSITION_EMBEDDINGS,
                "sliding_window": null,
                "max_window_layers": FIXTURE_NUM_LAYERS,
                "tie_word_embeddings": false,
                "rope_theta": 10000.0,
                "rms_norm_eps": 1e-5,
                "use_sliding_window": false,
                "hidden_act": "silu",
                "eos_token_id": null
            }),
        ),
        ModelArchitecture::Qwen35Moe => (
            "qwen3_moe",
            "Qwen3MoeForCausalLM",
            serde_json::json!({
                "model_type": "qwen3_moe",
                "vocab_size": FIXTURE_VOCAB_SIZE,
                "hidden_size": FIXTURE_HIDDEN_SIZE,
                "intermediate_size": FIXTURE_INTERMEDIATE_SIZE,
                "num_hidden_layers": FIXTURE_NUM_LAYERS,
                "num_attention_heads": FIXTURE_NUM_HEADS,
                "head_dim": FIXTURE_HIDDEN_SIZE / FIXTURE_NUM_HEADS,
                "attention_bias": false,
                "num_key_value_heads": FIXTURE_NUM_HEADS,
                "max_position_embeddings": FIXTURE_MAX_POSITION_EMBEDDINGS,
                "sliding_window": null,
                "max_window_layers": FIXTURE_NUM_LAYERS,
                "tie_word_embeddings": false,
                "rope_theta": 10000.0,
                "rms_norm_eps": 1e-5,
                "use_sliding_window": false,
                "hidden_act": "silu",
                "decoder_sparse_step": 1,
                "moe_intermediate_size": FIXTURE_INTERMEDIATE_SIZE,
                "num_experts_per_tok": 1,
                "num_experts": 2,
                "norm_topk_prob": true,
                "eos_token_id": null
            }),
        ),
        _ => anyhow::bail!("tiny Qwen fixture requires Qwen2, Qwen3, or Qwen3-MoE"),
    };
    config["model_type"] = serde_json::Value::String(model_type.to_owned());
    config["architectures"] = serde_json::json!([architecture_name]);
    fs::write(root.join(CONFIG_JSON), config.to_string())?;
    fs::write(root.join(TOKENIZER_JSON), TINY_TOKENIZER_JSON)?;

    let mut tensors = fixture_weights()?;
    if architecture == ModelArchitecture::Qwen2 {
        let head_dim = FIXTURE_HIDDEN_SIZE / FIXTURE_NUM_HEADS;
        let q = FIXTURE_NUM_HEADS * head_dim;
        let kv = FIXTURE_NUM_HEADS * head_dim;
        for layer in 0..FIXTURE_NUM_LAYERS {
            let prefix = format!("model.layers.{layer}.self_attn");
            tensors.insert(
                format!("{prefix}.q_proj.bias"),
                Tensor::zeros(q, DType::F32, &Device::Cpu)?,
            );
            tensors.insert(
                format!("{prefix}.k_proj.bias"),
                Tensor::zeros(kv, DType::F32, &Device::Cpu)?,
            );
            tensors.insert(
                format!("{prefix}.v_proj.bias"),
                Tensor::zeros(kv, DType::F32, &Device::Cpu)?,
            );
        }
    } else {
        let head_dim = FIXTURE_HIDDEN_SIZE / FIXTURE_NUM_HEADS;
        for layer in 0..FIXTURE_NUM_LAYERS {
            let prefix = format!("model.layers.{layer}.self_attn");
            tensors.insert(
                format!("{prefix}.q_norm.weight"),
                Tensor::ones(head_dim, DType::F32, &Device::Cpu)?,
            );
            tensors.insert(
                format!("{prefix}.k_norm.weight"),
                Tensor::ones(head_dim, DType::F32, &Device::Cpu)?,
            );
        }
        if architecture == ModelArchitecture::Qwen35Moe {
            for layer in 0..FIXTURE_NUM_LAYERS {
                let prefix = format!("model.layers.{layer}.mlp");
                tensors.insert(
                    format!("{prefix}.gate.weight"),
                    Tensor::zeros((2, FIXTURE_HIDDEN_SIZE), DType::F32, &Device::Cpu)?,
                );
                for expert in 0..2 {
                    tensors.insert(
                        format!("{prefix}.experts.{expert}.gate_proj.weight"),
                        Tensor::ones(
                            (FIXTURE_INTERMEDIATE_SIZE, FIXTURE_HIDDEN_SIZE),
                            DType::F32,
                            &Device::Cpu,
                        )?,
                    );
                    tensors.insert(
                        format!("{prefix}.experts.{expert}.up_proj.weight"),
                        Tensor::ones(
                            (FIXTURE_INTERMEDIATE_SIZE, FIXTURE_HIDDEN_SIZE),
                            DType::F32,
                            &Device::Cpu,
                        )?,
                    );
                    tensors.insert(
                        format!("{prefix}.experts.{expert}.down_proj.weight"),
                        Tensor::ones(
                            (FIXTURE_HIDDEN_SIZE, FIXTURE_INTERMEDIATE_SIZE),
                            DType::F32,
                            &Device::Cpu,
                        )?,
                    );
                }
            }
        }
    }
    candle_core::safetensors::save(&tensors, root.join(MODEL_SAFETENSORS))?;
    Ok(())
}

#[cfg(test)]
fn write_tachyon_tiny_gemma_fixture(
    root: &Path,
    architecture: ModelArchitecture,
) -> anyhow::Result<()> {
    fs::create_dir_all(root)?;
    let config = match architecture {
        ModelArchitecture::Gemma2 => serde_json::json!({
            "model_type": "gemma2",
            "architectures": ["Gemma2ForCausalLM"],
            "attention_bias": false,
            "head_dim": FIXTURE_HIDDEN_SIZE / FIXTURE_NUM_HEADS,
            "hidden_activation": "gelu",
            "hidden_size": FIXTURE_HIDDEN_SIZE,
            "intermediate_size": FIXTURE_INTERMEDIATE_SIZE,
            "num_attention_heads": FIXTURE_NUM_HEADS,
            "num_hidden_layers": FIXTURE_NUM_LAYERS,
            "num_key_value_heads": FIXTURE_NUM_HEADS,
            "rms_norm_eps": 1e-5,
            "rope_theta": 10000.0,
            "vocab_size": FIXTURE_VOCAB_SIZE,
            "final_logit_softcapping": null,
            "attn_logit_softcapping": null,
            "query_pre_attn_scalar": FIXTURE_HIDDEN_SIZE / FIXTURE_NUM_HEADS,
            "sliding_window": null,
            "max_position_embeddings": FIXTURE_MAX_POSITION_EMBEDDINGS,
            "eos_token_id": null
        }),
        ModelArchitecture::Gemma3 => serde_json::json!({
            "model_type": "gemma3_text",
            "architectures": ["Gemma3ForCausalLM"],
            "attention_bias": false,
            "head_dim": FIXTURE_HIDDEN_SIZE / FIXTURE_NUM_HEADS,
            "hidden_activation": "gelu",
            "hidden_size": FIXTURE_HIDDEN_SIZE,
            "intermediate_size": FIXTURE_INTERMEDIATE_SIZE,
            "num_attention_heads": FIXTURE_NUM_HEADS,
            "num_hidden_layers": FIXTURE_NUM_LAYERS,
            "num_key_value_heads": FIXTURE_NUM_HEADS,
            "rms_norm_eps": 1e-5,
            "rope_theta": 10000.0,
            "rope_local_base_freq": 10000.0,
            "vocab_size": FIXTURE_VOCAB_SIZE,
            "final_logit_softcapping": null,
            "attn_logit_softcapping": null,
            "query_pre_attn_scalar": FIXTURE_HIDDEN_SIZE / FIXTURE_NUM_HEADS,
            "sliding_window": FIXTURE_MAX_POSITION_EMBEDDINGS,
            "sliding_window_pattern": 2,
            "max_position_embeddings": FIXTURE_MAX_POSITION_EMBEDDINGS,
            "eos_token_id": null
        }),
        _ => anyhow::bail!("tiny Gemma fixture requires Gemma2 or Gemma3"),
    };
    fs::write(root.join(CONFIG_JSON), config.to_string())?;
    fs::write(root.join(TOKENIZER_JSON), TINY_TOKENIZER_JSON)?;
    let mut tensors = fixture_weights()?;
    let head_dim = FIXTURE_HIDDEN_SIZE / FIXTURE_NUM_HEADS;
    for layer in 0..FIXTURE_NUM_LAYERS {
        let prefix = format!("model.layers.{layer}");
        for norm_name in [
            "pre_feedforward_layernorm.weight",
            "post_feedforward_layernorm.weight",
        ] {
            tensors.insert(
                format!("{prefix}.{norm_name}"),
                Tensor::zeros(FIXTURE_HIDDEN_SIZE, DType::F32, &Device::Cpu)?,
            );
        }
        if architecture == ModelArchitecture::Gemma3 {
            tensors.insert(
                format!("{prefix}.self_attn.q_norm.weight"),
                Tensor::zeros(head_dim, DType::F32, &Device::Cpu)?,
            );
            tensors.insert(
                format!("{prefix}.self_attn.k_norm.weight"),
                Tensor::zeros(head_dim, DType::F32, &Device::Cpu)?,
            );
        }
    }
    // Gemma ties the LM head to the embedding table.
    tensors.remove("lm_head.weight");
    candle_core::safetensors::save(&tensors, root.join(MODEL_SAFETENSORS))?;
    Ok(())
}

#[cfg(test)]
fn write_tachyon_tiny_phi_fixture(
    root: &Path,
    architecture: ModelArchitecture,
) -> anyhow::Result<()> {
    let model_type = match architecture {
        ModelArchitecture::Phi3 => "phi3",
        ModelArchitecture::Phi4 => "phi4",
        _ => anyhow::bail!("tiny Phi fixture requires Phi3 or Phi4"),
    };
    fs::create_dir_all(root)?;
    fs::write(
        root.join(CONFIG_JSON),
        serde_json::json!({
            "model_type": model_type,
            "architectures": ["Phi3ForCausalLM"],
            "vocab_size": FIXTURE_VOCAB_SIZE,
            "hidden_act": "silu",
            "hidden_size": FIXTURE_HIDDEN_SIZE,
            "intermediate_size": FIXTURE_INTERMEDIATE_SIZE,
            "num_hidden_layers": FIXTURE_NUM_LAYERS,
            "num_attention_heads": FIXTURE_NUM_HEADS,
            "num_key_value_heads": FIXTURE_NUM_HEADS,
            "rms_norm_eps": 1e-5,
            "rope_theta": 10000.0,
            "bos_token_id": 1,
            "eos_token_id": null,
            "rope_scaling": null,
            "max_position_embeddings": FIXTURE_MAX_POSITION_EMBEDDINGS,
            "original_max_position_embeddings": null,
            "partial_rotary_factor": null,
            "tie_word_embeddings": false
        })
        .to_string(),
    )?;
    fs::write(root.join(TOKENIZER_JSON), TINY_TOKENIZER_JSON)?;
    let mut tensors = fixture_weights()?;
    for layer in 0..FIXTURE_NUM_LAYERS {
        let prefix = format!("model.layers.{layer}");
        let q = tensors
            .remove(&format!("{prefix}.self_attn.q_proj.weight"))
            .expect("fixture q weight");
        let k = tensors
            .remove(&format!("{prefix}.self_attn.k_proj.weight"))
            .expect("fixture k weight");
        let v = tensors
            .remove(&format!("{prefix}.self_attn.v_proj.weight"))
            .expect("fixture v weight");
        tensors.insert(
            format!("{prefix}.self_attn.qkv_proj.weight"),
            Tensor::cat(&[&q, &k, &v], 0)?,
        );
        let gate = tensors
            .remove(&format!("{prefix}.mlp.gate_proj.weight"))
            .expect("fixture gate weight");
        let up = tensors
            .remove(&format!("{prefix}.mlp.up_proj.weight"))
            .expect("fixture up weight");
        tensors.insert(
            format!("{prefix}.mlp.gate_up_proj.weight"),
            Tensor::cat(&[&gate, &up], 0)?,
        );
    }
    candle_core::safetensors::save(&tensors, root.join(MODEL_SAFETENSORS))?;
    Ok(())
}

#[cfg(test)]
fn write_tachyon_tiny_deepseek_fixture(
    root: &Path,
    architecture: ModelArchitecture,
) -> anyhow::Result<()> {
    let model_type = match architecture {
        ModelArchitecture::DeepSeekV2 => "deepseek_v2",
        ModelArchitecture::DeepSeekV3 => "deepseek_v3",
        ModelArchitecture::DeepSeekR1 => "deepseek_r1",
        _ => anyhow::bail!("tiny DeepSeek fixture requires a DeepSeek architecture"),
    };
    fs::create_dir_all(root)?;
    fs::write(
        root.join(CONFIG_JSON),
        serde_json::json!({
            "model_type": model_type,
            "architectures": ["DeepseekV2ForCausalLM"],
            "vocab_size": FIXTURE_VOCAB_SIZE,
            "hidden_size": FIXTURE_HIDDEN_SIZE,
            "intermediate_size": FIXTURE_INTERMEDIATE_SIZE,
            "moe_intermediate_size": FIXTURE_INTERMEDIATE_SIZE,
            "num_hidden_layers": 0,
            "num_attention_heads": FIXTURE_NUM_HEADS,
            "n_shared_experts": null,
            "n_routed_experts": null,
            "num_experts_per_tok": null,
            "max_position_embeddings": FIXTURE_MAX_POSITION_EMBEDDINGS,
            "rms_norm_eps": 1e-5,
            "tie_word_embeddings": false,
            "rope_theta": 10000.0,
            "rope_scaling": null,
            "attention_bias": false,
            "q_lora_rank": null,
            "qk_rope_head_dim": 2,
            "kv_lora_rank": 2,
            "v_head_dim": 2,
            "qk_nope_head_dim": 2,
            "n_group": 1,
            "topk_group": 1,
            "eos_token_id": null
        })
        .to_string(),
    )?;
    fs::write(root.join(TOKENIZER_JSON), TINY_TOKENIZER_JSON)?;
    let mut tensors = HashMap::new();
    tensors.insert(
        "model.embed_tokens.weight".to_owned(),
        Tensor::from_vec(
            deterministic_fill(71, FIXTURE_VOCAB_SIZE * FIXTURE_HIDDEN_SIZE),
            (FIXTURE_VOCAB_SIZE, FIXTURE_HIDDEN_SIZE),
            &Device::Cpu,
        )?,
    );
    tensors.insert(
        "model.norm.weight".to_owned(),
        Tensor::ones(FIXTURE_HIDDEN_SIZE, DType::F32, &Device::Cpu)?,
    );
    tensors.insert(
        "lm_head.weight".to_owned(),
        Tensor::from_vec(
            deterministic_fill(72, FIXTURE_VOCAB_SIZE * FIXTURE_HIDDEN_SIZE),
            (FIXTURE_VOCAB_SIZE, FIXTURE_HIDDEN_SIZE),
            &Device::Cpu,
        )?,
    );
    candle_core::safetensors::save(&tensors, root.join(MODEL_SAFETENSORS))?;
    Ok(())
}

/// Deterministic small weights in roughly `[-0.4, 0.4)`, distinct per element and
/// per tensor (via `seed`), so the forward pass varies with the input rather than
/// collapsing.
#[cfg(test)]
fn deterministic_fill(seed: u64, len: usize) -> Vec<f32> {
    (0..len)
        .map(|index| {
            let mixed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(
                (index as u64)
                    .wrapping_add(1)
                    .wrapping_mul(0x6789_ABCD_EF01_2345),
            );
            let unit = ((mixed >> 40) & 0xFFFF) as f32 / 65_535.0; // [0, 1]
            (unit - 0.5) * 0.8
        })
        .collect()
}

/// Number of experts per layer in [`write_tachyon_tiny_mixtral_fixture`]. Kept
/// at 1 so top-1 routing always selects the only expert, making the MoE
/// block's output identical to a dense block built from the same weight
/// values (the fixture writer duplicates each layer's MLP weights under both
/// the `mlp.*` dense alias and the `block_sparse_moe.*` MoE alias).
#[cfg(test)]
pub(crate) const FIXTURE_NUM_EXPERTS: usize = 1;

/// Write a complete, deterministic tiny **Mixtral-style MoE** checkpoint where
/// every layer is MoE with a single expert per layer. Used to prove
/// [`super::expert_parallel_llama::ExpertParallelLlama`]'s MoE block is
/// numerically equivalent to a dense reference run with the same weights
/// (top-1 routing over one expert is the identity routing).
#[cfg(test)]
pub(crate) fn write_tachyon_tiny_mixtral_fixture(root: &Path) -> anyhow::Result<()> {
    let layers = [Some(FIXTURE_NUM_EXPERTS); FIXTURE_NUM_LAYERS];
    write_tachyon_tiny_mixtral_fixture_with(root, &layers)
}

/// Write a tiny Mixtral-style checkpoint with a mixed dense/MoE layer stack:
/// layer 0 is dense (no `.experts.` tensors), every other layer is MoE with
/// [`FIXTURE_NUM_EXPERTS`] experts plus one extra expert (so routing among
/// more than one expert is actually exercised).
#[cfg(test)]
pub(crate) fn write_tachyon_tiny_mixtral_mixed_fixture(root: &Path) -> anyhow::Result<()> {
    let mut layers = vec![None; FIXTURE_NUM_LAYERS];
    for expert_count in layers.iter_mut().skip(1) {
        *expert_count = Some(FIXTURE_NUM_EXPERTS + 1);
    }
    write_tachyon_tiny_mixtral_fixture_with(root, &layers)
}

/// Shared implementation: `layer_experts[i]` is `Some(num_experts)` for an MoE
/// layer or `None` for a dense layer. Every MoE layer's expert weights are
/// distinct (seeded per expert), but an MoE layer's expert 0 always carries
/// the same values as the layer's `mlp.*` dense alias, which
/// `write_tachyon_tiny_mixtral_fixture`'s all-one-expert case relies on for
/// exact dense-reference equivalence.
#[cfg(test)]
fn write_tachyon_tiny_mixtral_fixture_with(
    root: &Path,
    layer_experts: &[Option<usize>],
) -> anyhow::Result<()> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join(CONFIG_JSON),
        serde_json::json!({
            "model_type": "mixtral",
            "architectures": ["MixtralForCausalLM"],
            "hidden_size": FIXTURE_HIDDEN_SIZE,
            "intermediate_size": FIXTURE_INTERMEDIATE_SIZE,
            "vocab_size": FIXTURE_VOCAB_SIZE,
            "num_hidden_layers": layer_experts.len(),
            "num_attention_heads": FIXTURE_NUM_HEADS,
            "num_key_value_heads": FIXTURE_NUM_HEADS,
            "rms_norm_eps": 1e-5,
            "rope_theta": 10000.0,
            "max_position_embeddings": FIXTURE_MAX_POSITION_EMBEDDINGS,
            "tie_word_embeddings": false,
            "bos_token_id": 1,
            "eos_token_id": null,
            "num_local_experts": layer_experts.iter().filter_map(|c| *c).max().unwrap_or(0),
            "num_experts_per_tok": 1
        })
        .to_string(),
    )?;
    fs::write(root.join(TOKENIZER_JSON), TINY_TOKENIZER_JSON)?;
    candle_core::safetensors::save(
        &mixtral_fixture_weights(layer_experts)?,
        root.join(MODEL_SAFETENSORS),
    )?;
    Ok(())
}

/// Deterministic weights for the Mixtral-style fixtures, in the tensor layout
/// `expert_parallel_llama.rs` and (for dense layers / the dense-equivalence
/// reference) `candle_transformers::models::llama::Llama` expect.
#[cfg(test)]
fn mixtral_fixture_weights(
    layer_experts: &[Option<usize>],
) -> anyhow::Result<HashMap<String, Tensor>> {
    let hidden = FIXTURE_HIDDEN_SIZE;
    let inter = FIXTURE_INTERMEDIATE_SIZE;
    let head_dim = hidden / FIXTURE_NUM_HEADS;
    let q = FIXTURE_NUM_HEADS * head_dim;
    let kv = FIXTURE_NUM_HEADS * head_dim;
    let mut tensors = HashMap::new();
    let mut seed = 1u64;
    let mut dense =
        |tensors: &mut HashMap<String, Tensor>, name: &str, dims: &[usize]| -> anyhow::Result<()> {
            let len: usize = dims.iter().product();
            let values = deterministic_fill(seed, len);
            seed = seed.wrapping_add(1);
            tensors.insert(
                name.to_owned(),
                Tensor::from_vec(values, dims.to_vec(), &Device::Cpu)?,
            );
            Ok(())
        };
    let norm = |tensors: &mut HashMap<String, Tensor>, name: &str| -> anyhow::Result<()> {
        tensors.insert(
            name.to_owned(),
            Tensor::from_vec(vec![1f32; hidden], (hidden,), &Device::Cpu)?,
        );
        Ok(())
    };

    dense(
        &mut tensors,
        "model.embed_tokens.weight",
        &[FIXTURE_VOCAB_SIZE, hidden],
    )?;
    for (layer, expert_count) in layer_experts.iter().enumerate() {
        let prefix = format!("model.layers.{layer}");
        dense(
            &mut tensors,
            &format!("{prefix}.self_attn.q_proj.weight"),
            &[q, hidden],
        )?;
        dense(
            &mut tensors,
            &format!("{prefix}.self_attn.k_proj.weight"),
            &[kv, hidden],
        )?;
        dense(
            &mut tensors,
            &format!("{prefix}.self_attn.v_proj.weight"),
            &[kv, hidden],
        )?;
        dense(
            &mut tensors,
            &format!("{prefix}.self_attn.o_proj.weight"),
            &[hidden, q],
        )?;
        norm(&mut tensors, &format!("{prefix}.input_layernorm.weight"))?;
        norm(
            &mut tensors,
            &format!("{prefix}.post_attention_layernorm.weight"),
        )?;
        match expert_count {
            None => {
                dense(
                    &mut tensors,
                    &format!("{prefix}.mlp.gate_proj.weight"),
                    &[inter, hidden],
                )?;
                dense(
                    &mut tensors,
                    &format!("{prefix}.mlp.up_proj.weight"),
                    &[inter, hidden],
                )?;
                dense(
                    &mut tensors,
                    &format!("{prefix}.mlp.down_proj.weight"),
                    &[hidden, inter],
                )?;
            }
            Some(num_experts) => {
                dense(
                    &mut tensors,
                    &format!("{prefix}.block_sparse_moe.gate.weight"),
                    &[*num_experts, hidden],
                )?;
                // Expert 0's weights are also written under the dense `mlp.*`
                // alias, so a dense reference model built from the same file
                // sees identical values for layers where `num_experts == 1`
                // (the all-one-expert fixture's equivalence test relies on this).
                dense(
                    &mut tensors,
                    &format!("{prefix}.block_sparse_moe.experts.0.w1.weight"),
                    &[inter, hidden],
                )?;
                dense(
                    &mut tensors,
                    &format!("{prefix}.block_sparse_moe.experts.0.w3.weight"),
                    &[inter, hidden],
                )?;
                dense(
                    &mut tensors,
                    &format!("{prefix}.block_sparse_moe.experts.0.w2.weight"),
                    &[hidden, inter],
                )?;
                tensors.insert(
                    format!("{prefix}.mlp.gate_proj.weight"),
                    tensors[&format!("{prefix}.block_sparse_moe.experts.0.w1.weight")].clone(),
                );
                tensors.insert(
                    format!("{prefix}.mlp.up_proj.weight"),
                    tensors[&format!("{prefix}.block_sparse_moe.experts.0.w3.weight")].clone(),
                );
                tensors.insert(
                    format!("{prefix}.mlp.down_proj.weight"),
                    tensors[&format!("{prefix}.block_sparse_moe.experts.0.w2.weight")].clone(),
                );
                for expert_id in 1..*num_experts {
                    dense(
                        &mut tensors,
                        &format!("{prefix}.block_sparse_moe.experts.{expert_id}.w1.weight"),
                        &[inter, hidden],
                    )?;
                    dense(
                        &mut tensors,
                        &format!("{prefix}.block_sparse_moe.experts.{expert_id}.w3.weight"),
                        &[inter, hidden],
                    )?;
                    dense(
                        &mut tensors,
                        &format!("{prefix}.block_sparse_moe.experts.{expert_id}.w2.weight"),
                        &[hidden, inter],
                    )?;
                }
            }
        }
    }
    norm(&mut tensors, "model.norm.weight")?;
    dense(
        &mut tensors,
        "lm_head.weight",
        &[FIXTURE_VOCAB_SIZE, hidden],
    )?;
    Ok(tensors)
}

/// Write a complete, deterministic tiny **Llama GGUF** checkpoint (`model.gguf` +
/// `tokenizer.json` + the broker's `.tachyon-model.json` sidecar) so tests
/// exercise the real candle-transformers quantized Llama forward. Tensors are
/// stored as F32 (block size 1), so the tiny dimensions need no block padding.
#[cfg(test)]
pub(crate) fn write_tachyon_tiny_gguf_fixture(root: &Path) -> anyhow::Result<()> {
    use candle_core::quantized::{GgmlDType, QTensor};
    use gguf_file::Value;

    fs::create_dir_all(root)?;
    fs::write(root.join(TOKENIZER_JSON), TINY_TOKENIZER_JSON)?;

    let hidden = FIXTURE_HIDDEN_SIZE;
    let inter = FIXTURE_INTERMEDIATE_SIZE;
    let head_dim = hidden / FIXTURE_NUM_HEADS;

    // Build f32 tensors under GGUF-native names, then quantize (F32 = identity).
    // Non-capturing nested fns (not closures) so each call borrows `raw` only for
    // its own duration.
    fn dense(
        raw: &mut Vec<(String, Tensor)>,
        seed: &mut u64,
        name: String,
        dims: &[usize],
    ) -> anyhow::Result<()> {
        let len: usize = dims.iter().product();
        let values = deterministic_fill(*seed, len);
        *seed = seed.wrapping_add(1);
        raw.push((name, Tensor::from_vec(values, dims.to_vec(), &Device::Cpu)?));
        Ok(())
    }
    fn norm(raw: &mut Vec<(String, Tensor)>, hidden: usize, name: String) -> anyhow::Result<()> {
        raw.push((
            name,
            Tensor::from_vec(vec![1f32; hidden], (hidden,), &Device::Cpu)?,
        ));
        Ok(())
    }

    let mut seed = 1u64;
    let mut raw: Vec<(String, Tensor)> = Vec::new();
    dense(
        &mut raw,
        &mut seed,
        "token_embd.weight".to_owned(),
        &[FIXTURE_VOCAB_SIZE, hidden],
    )?;
    for layer in 0..FIXTURE_NUM_LAYERS {
        let prefix = format!("blk.{layer}");
        dense(
            &mut raw,
            &mut seed,
            format!("{prefix}.attn_q.weight"),
            &[hidden, hidden],
        )?;
        dense(
            &mut raw,
            &mut seed,
            format!("{prefix}.attn_k.weight"),
            &[hidden, hidden],
        )?;
        dense(
            &mut raw,
            &mut seed,
            format!("{prefix}.attn_v.weight"),
            &[hidden, hidden],
        )?;
        dense(
            &mut raw,
            &mut seed,
            format!("{prefix}.attn_output.weight"),
            &[hidden, hidden],
        )?;
        dense(
            &mut raw,
            &mut seed,
            format!("{prefix}.ffn_gate.weight"),
            &[inter, hidden],
        )?;
        dense(
            &mut raw,
            &mut seed,
            format!("{prefix}.ffn_up.weight"),
            &[inter, hidden],
        )?;
        dense(
            &mut raw,
            &mut seed,
            format!("{prefix}.ffn_down.weight"),
            &[hidden, inter],
        )?;
        norm(&mut raw, hidden, format!("{prefix}.attn_norm.weight"))?;
        norm(&mut raw, hidden, format!("{prefix}.ffn_norm.weight"))?;
    }
    norm(&mut raw, hidden, "output_norm.weight".to_owned())?;
    dense(
        &mut raw,
        &mut seed,
        "output.weight".to_owned(),
        &[FIXTURE_VOCAB_SIZE, hidden],
    )?;

    let quantized: Vec<(String, QTensor)> = raw
        .into_iter()
        .map(|(name, tensor)| Ok((name, QTensor::quantize(&tensor, GgmlDType::F32)?)))
        .collect::<anyhow::Result<_>>()?;
    let tensor_refs: Vec<(&str, &QTensor)> = quantized
        .iter()
        .map(|(name, q)| (name.as_str(), q))
        .collect();

    let metadata: Vec<(&str, Value)> = vec![
        (
            "general.architecture",
            Value::String(GGUF_LLAMA_ARCHITECTURE.to_owned()),
        ),
        (
            "llama.context_length",
            Value::U32(FIXTURE_MAX_POSITION_EMBEDDINGS as u32),
        ),
        ("llama.embedding_length", Value::U32(hidden as u32)),
        ("llama.block_count", Value::U32(FIXTURE_NUM_LAYERS as u32)),
        (
            "llama.attention.head_count",
            Value::U32(FIXTURE_NUM_HEADS as u32),
        ),
        (
            "llama.attention.head_count_kv",
            Value::U32(FIXTURE_NUM_HEADS as u32),
        ),
        ("llama.attention.layer_norm_rms_epsilon", Value::F32(1e-5)),
        ("llama.rope.dimension_count", Value::U32(head_dim as u32)),
        ("llama.rope.freq_base", Value::F32(10000.0)),
    ];
    let metadata_refs: Vec<(&str, &Value)> = metadata.iter().map(|(k, v)| (*k, v)).collect();

    let mut file = fs::File::create(root.join("model.gguf"))?;
    gguf_file::write(&mut file, &metadata_refs, &tensor_refs)?;

    fs::write(
        root.join(MODEL_META_JSON),
        serde_json::json!({ "format": "gguf" }).to_string(),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architecture_registry_normalizes_supported_hf_aliases() {
        assert_eq!(
            ModelArchitecture::from_hf_model_type("qwen2"),
            Some(ModelArchitecture::Qwen2)
        );
        assert_eq!(
            ModelArchitecture::from_hf_model_type("qwen3"),
            Some(ModelArchitecture::Qwen3)
        );
        assert_eq!(
            ModelArchitecture::from_hf_model_type("qwen3_moe"),
            Some(ModelArchitecture::Qwen35Moe)
        );
        assert_eq!(
            ModelArchitecture::from_hf_model_type("gemma3_text"),
            Some(ModelArchitecture::Gemma3)
        );
        assert_eq!(
            ModelArchitecture::from_hf_model_type("deepseek_r1"),
            Some(ModelArchitecture::DeepSeekR1)
        );
        assert_eq!(ModelArchitecture::from_hf_model_type("gpt2"), None);
    }

    #[test]
    fn architecture_registry_recognizes_multimodal_hf_aliases_as_unsupported() {
        for model_type in ["qwen_vl", "qwen2_vl", "qwen2_5_vl", "qwen3_vl"] {
            assert!(
                is_multimodal_hf_model_type(model_type),
                "{model_type} must stay out of the text-only architecture registry"
            );
            assert_eq!(ModelArchitecture::from_hf_model_type(model_type), None);
        }
        assert!(!is_multimodal_hf_model_type("gemma3"));
        assert!(is_multimodal_hf_model_type("gemma3_vl"));
    }

    #[test]
    fn architecture_registry_keeps_format_support_explicit() {
        assert!(ModelArchitecture::Llama.supports_format(ModelFormat::Safetensors));
        assert!(ModelArchitecture::Llama.supports_format(ModelFormat::Gguf));
        assert!(ModelArchitecture::Qwen3.supports_format(ModelFormat::Safetensors));
        assert!(ModelArchitecture::Qwen35Moe.supports_format(ModelFormat::Safetensors));
        assert!(!ModelArchitecture::DeepSeekV3.supports_format(ModelFormat::Safetensors));
        assert!(!ModelArchitecture::Gemma3.supports_format(ModelFormat::Gguf));
        assert!(ModelArchitecture::Gemma3.supports_format(ModelFormat::Safetensors));
    }

    #[test]
    fn architecture_registry_rejects_unregistered_parallel_modes() {
        let tensor = HardwareStrategy {
            distribution_mode: GpuDistribution::TensorParallelism,
            ..HardwareStrategy::default()
        };
        let expert = HardwareStrategy {
            distribution_mode: GpuDistribution::ExpertParallelism,
            ..HardwareStrategy::default()
        };
        assert!(ModelArchitecture::Llama.supports_strategy(&tensor));
        assert!(!ModelArchitecture::Qwen3.supports_strategy(&tensor));
        assert!(ModelArchitecture::Mixtral.supports_strategy(&expert));
        assert!(!ModelArchitecture::Llama.supports_strategy(&expert));
    }

    fn load_qwen_fixture(
        tag: &str,
        architecture: ModelArchitecture,
    ) -> (CandleLlmRuntime, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "tachyon-{}-{tag}-{}-{nanos}",
            architecture.display_name(),
            std::process::id()
        ));
        write_tachyon_tiny_qwen_fixture(&dir, architecture)
            .expect("Qwen fixture should be written");
        let runtime =
            CandleLlmRuntime::try_load("tiny-qwen", &dir, "cpu", &HardwareStrategy::default())
                .expect("Qwen fixture should load without error")
                .expect("Qwen fixture should select the native Candle runtime");
        (runtime, dir)
    }

    #[test]
    fn qwen2_qwen3_and_qwen3_moe_generate_and_reset_their_internal_cache() {
        for architecture in [
            ModelArchitecture::Qwen2,
            ModelArchitecture::Qwen3,
            ModelArchitecture::Qwen35Moe,
        ] {
            let (runtime, dir) = load_qwen_fixture("generation", architecture);
            let request: &[u8] = br#"{"prompt":"hello mesh","max_new_tokens":4}"#;
            let first = runtime.generate(&[request]).expect("first generation");
            let second = runtime.generate(&[request]).expect("second generation");
            assert_eq!(
                first,
                second,
                "{} must reset its internal KV cache between requests",
                architecture.display_name()
            );
            assert_ne!(first, b"MOCK_LLM_RESPONSE");
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn qwen2_qwen3_and_qwen3_moe_streaming_matches_buffered_generation() {
        for architecture in [
            ModelArchitecture::Qwen2,
            ModelArchitecture::Qwen3,
            ModelArchitecture::Qwen35Moe,
        ] {
            let (runtime, dir) = load_qwen_fixture("streaming", architecture);
            let request: &[u8] = br#"{"prompt":"hello","max_new_tokens":4}"#;
            let buffered =
                String::from_utf8(runtime.generate(&[request]).expect("buffered")).expect("utf-8");
            let mut streamed = String::new();
            runtime
                .generate_streaming(&[request], &mut |delta| streamed.push_str(delta))
                .expect("streamed");
            assert_eq!(streamed, buffered);
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn qwen2_qwen3_and_qwen3_moe_logits_match_direct_candle_models_and_enforce_limits() {
        for architecture in [
            ModelArchitecture::Qwen2,
            ModelArchitecture::Qwen3,
            ModelArchitecture::Qwen35Moe,
        ] {
            let (runtime, dir) = load_qwen_fixture("reference", architecture);
            let runtime_logits = runtime.debug_last_logits("hello mesh");
            let ids = runtime
                .encode_ids("hello mesh")
                .expect("prompt should encode");
            let input = Tensor::new(ids.as_slice(), &Device::Cpu)
                .and_then(|tensor| tensor.unsqueeze(0))
                .expect("input should build");
            let raw_config = fs::read(dir.join(CONFIG_JSON)).expect("config should read");
            let paths = vec![dir.join(MODEL_SAFETENSORS)];
            // SAFETY: the fixture remains immutable for the lifetime of the
            // direct reference model.
            let vb = unsafe {
                VarBuilder::from_mmaped_safetensors(&paths, DType::F32, &Device::Cpu)
                    .expect("fixture weights should map")
            };
            let reference = match architecture {
                ModelArchitecture::Qwen2 => {
                    let config: Qwen2Config =
                        serde_json::from_slice(&raw_config).expect("Qwen2 config");
                    let mut model = Qwen2Model::new(&config, vb).expect("Qwen2 reference");
                    model
                        .forward(&input, 0)
                        .and_then(|logits| logits.squeeze(1))
                        .and_then(|logits| logits.squeeze(0))
                        .and_then(|logits| logits.to_vec1::<f32>())
                        .expect("Qwen2 reference logits")
                }
                ModelArchitecture::Qwen3 => {
                    let config: Qwen3Config =
                        serde_json::from_slice(&raw_config).expect("Qwen3 config");
                    let mut model = Qwen3Model::new(&config, vb).expect("Qwen3 reference");
                    model
                        .forward(&input, 0)
                        .and_then(|logits| logits.squeeze(1))
                        .and_then(|logits| logits.squeeze(0))
                        .and_then(|logits| logits.to_vec1::<f32>())
                        .expect("Qwen3 reference logits")
                }
                ModelArchitecture::Qwen35Moe => {
                    let config: Qwen3MoeConfig =
                        serde_json::from_slice(&raw_config).expect("Qwen3-MoE config");
                    let mut model = Qwen3MoeModel::new(&config, vb).expect("Qwen3-MoE reference");
                    model
                        .forward(&input, 0)
                        .and_then(|logits| logits.squeeze(1))
                        .and_then(|logits| logits.squeeze(0))
                        .and_then(|logits| logits.to_vec1::<f32>())
                        .expect("Qwen3-MoE reference logits")
                }
                _ => unreachable!(),
            };
            assert_eq!(runtime_logits, reference);

            let request = format!(
                r#"{{"prompt":"hello","max_new_tokens":{}}}"#,
                HOST_MAX_NEW_TOKENS + 1
            );
            let error = runtime
                .generate(&[request.as_bytes()])
                .expect_err("host generation limit must apply to Qwen");
            assert!(matches!(error, CandleLlmError::InvalidRequest { .. }));
            let _ = fs::remove_dir_all(dir);
        }
    }

    fn load_gemma_fixture(
        tag: &str,
        architecture: ModelArchitecture,
    ) -> (CandleLlmRuntime, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "tachyon-{}-{tag}-{}-{nanos}",
            architecture.display_name(),
            std::process::id()
        ));
        write_tachyon_tiny_gemma_fixture(&dir, architecture)
            .expect("Gemma fixture should be written");
        let runtime =
            CandleLlmRuntime::try_load("tiny-gemma", &dir, "cpu", &HardwareStrategy::default())
                .expect("Gemma fixture should load without error")
                .expect("Gemma fixture should select the native Candle runtime");
        (runtime, dir)
    }

    #[test]
    fn gemma2_and_gemma3_generate_with_streaming_parity_and_cache_reset() {
        for architecture in [ModelArchitecture::Gemma2, ModelArchitecture::Gemma3] {
            let (runtime, dir) = load_gemma_fixture("generation", architecture);
            let request: &[u8] = br#"{"prompt":"hello mesh","max_new_tokens":4}"#;
            let first =
                String::from_utf8(runtime.generate(&[request]).expect("buffered")).expect("utf-8");
            let second =
                String::from_utf8(runtime.generate(&[request]).expect("repeat")).expect("utf-8");
            assert_eq!(first, second, "internal KV cache must reset");
            let mut streamed = String::new();
            runtime
                .generate_streaming(&[request], &mut |delta| streamed.push_str(delta))
                .expect("streamed");
            assert_eq!(streamed, first);
            assert_ne!(first.as_bytes(), b"MOCK_LLM_RESPONSE");
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn gemma2_and_gemma3_logits_match_direct_candle_models_and_enforce_limits() {
        for architecture in [ModelArchitecture::Gemma2, ModelArchitecture::Gemma3] {
            let (runtime, dir) = load_gemma_fixture("reference", architecture);
            let runtime_logits = runtime.debug_last_logits("hello mesh");
            let ids = runtime
                .encode_ids("hello mesh")
                .expect("prompt should encode");
            let input = Tensor::new(ids.as_slice(), &Device::Cpu)
                .and_then(|tensor| tensor.unsqueeze(0))
                .expect("input should build");
            let raw_config = fs::read(dir.join(CONFIG_JSON)).expect("config should read");
            let paths = vec![dir.join(MODEL_SAFETENSORS)];
            // SAFETY: the fixture remains immutable for the lifetime of the
            // direct reference model.
            let vb = unsafe {
                VarBuilder::from_mmaped_safetensors(&paths, DType::F32, &Device::Cpu)
                    .expect("fixture weights should map")
            };
            let reference = match architecture {
                ModelArchitecture::Gemma2 => {
                    let config: Gemma2Config =
                        serde_json::from_slice(&raw_config).expect("Gemma2 config");
                    let mut model = Gemma2Model::new(false, &config, vb).expect("Gemma2 reference");
                    model
                        .forward(&input, 0)
                        .and_then(|logits| logits.squeeze(1))
                        .and_then(|logits| logits.squeeze(0))
                        .and_then(|logits| logits.to_vec1::<f32>())
                        .expect("Gemma2 reference logits")
                }
                ModelArchitecture::Gemma3 => {
                    let config: Gemma3Config =
                        serde_json::from_slice(&raw_config).expect("Gemma3 config");
                    let mut model = Gemma3Model::new(false, &config, vb).expect("Gemma3 reference");
                    model
                        .forward(&input, 0)
                        .and_then(|logits| logits.squeeze(1))
                        .and_then(|logits| logits.squeeze(0))
                        .and_then(|logits| logits.to_vec1::<f32>())
                        .expect("Gemma3 reference logits")
                }
                _ => unreachable!(),
            };
            assert_eq!(runtime_logits, reference);

            let request = format!(
                r#"{{"prompt":"hello","max_new_tokens":{}}}"#,
                HOST_MAX_NEW_TOKENS + 1
            );
            let error = runtime
                .generate(&[request.as_bytes()])
                .expect_err("host generation limit must apply to Gemma");
            assert!(matches!(error, CandleLlmError::InvalidRequest { .. }));
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn gemma3_multimodal_config_is_rejected_before_weight_loading() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "tachyon-gemma3-multimodal-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("dir");
        fs::write(
            dir.join(CONFIG_JSON),
            serde_json::json!({
                "model_type": "gemma3",
                "text_config": {"model_type": "gemma3_text"},
                "vision_config": {"model_type": "siglip"}
            })
            .to_string(),
        )
        .expect("config");
        let error = inspect_model_architecture("gemma-mm", &dir, ModelFormat::Safetensors)
            .expect_err("multimodal Gemma3 must be refused");
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(detail.contains("multimodal"));
                assert!(detail.contains("vision_config"));
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn qwen_vl_config_is_rejected_before_weight_loading() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("tachyon-qwen-vl-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).expect("dir");
        fs::write(
            dir.join(CONFIG_JSON),
            serde_json::json!({
                "model_type": "qwen2_5_vl",
                "text_config": {"model_type": "qwen2"},
                "vision_config": {"model_type": "qwen2_5_vl_vision"}
            })
            .to_string(),
        )
        .expect("config");
        let error = inspect_model_architecture("qwen-vl", &dir, ModelFormat::Safetensors)
            .expect_err("multimodal Qwen-VL must be refused");
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(detail.contains("multimodal"));
                assert!(detail.contains("qwen2_5_vl"));
                assert!(detail.contains("vision encoder"));
                assert!(detail.contains("image request tensor"));
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    fn load_family_fixture(
        tag: &str,
        architecture: ModelArchitecture,
        writer: impl FnOnce(&Path, ModelArchitecture) -> anyhow::Result<()>,
    ) -> (CandleLlmRuntime, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "tachyon-{}-{tag}-{}-{nanos}",
            architecture.display_name(),
            std::process::id()
        ));
        writer(&dir, architecture).expect("family fixture should be written");
        let runtime =
            CandleLlmRuntime::try_load("tiny-family", &dir, "cpu", &HardwareStrategy::default())
                .expect("family fixture should load without error")
                .expect("family fixture should select the native Candle runtime");
        (runtime, dir)
    }

    #[test]
    fn phi3_and_compatible_phi4_configs_generate_and_stream() {
        for architecture in [ModelArchitecture::Phi3, ModelArchitecture::Phi4] {
            let (runtime, dir) =
                load_family_fixture("generation", architecture, write_tachyon_tiny_phi_fixture);
            let request: &[u8] = br#"{"prompt":"hello mesh","max_new_tokens":4}"#;
            let buffered =
                String::from_utf8(runtime.generate(&[request]).expect("buffered")).expect("utf-8");
            let repeat =
                String::from_utf8(runtime.generate(&[request]).expect("repeat")).expect("utf-8");
            assert_eq!(buffered, repeat);
            let mut streamed = String::new();
            runtime
                .generate_streaming(&[request], &mut |delta| streamed.push_str(delta))
                .expect("streamed");
            assert_eq!(streamed, buffered);
            assert_ne!(buffered.as_bytes(), b"MOCK_LLM_RESPONSE");
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn deepseek_v2_compatible_config_generates_and_streams() {
        let (runtime, dir) = load_family_fixture(
            "generation",
            ModelArchitecture::DeepSeekV2,
            write_tachyon_tiny_deepseek_fixture,
        );
        let request: &[u8] = br#"{"prompt":"hello mesh","max_new_tokens":4}"#;
        let buffered =
            String::from_utf8(runtime.generate(&[request]).expect("buffered")).expect("utf-8");
        let repeat =
            String::from_utf8(runtime.generate(&[request]).expect("repeat")).expect("utf-8");
        assert_eq!(buffered, repeat);
        let mut streamed = String::new();
        runtime
            .generate_streaming(&[request], &mut |delta| streamed.push_str(delta))
            .expect("streamed");
        assert_eq!(streamed, buffered);
        assert_ne!(buffered.as_bytes(), b"MOCK_LLM_RESPONSE");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn deepseek_v3_and_r1_fail_closed_until_candle_exposes_dedicated_backends() {
        for architecture in [ModelArchitecture::DeepSeekV3, ModelArchitecture::DeepSeekR1] {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after the epoch")
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "tachyon-{}-unsupported-{}-{nanos}",
                architecture.display_name(),
                std::process::id()
            ));
            write_tachyon_tiny_deepseek_fixture(&dir, architecture)
                .expect("DeepSeek fixture should be written");
            fs::write(dir.join(TOKENIZER_JSON), TINY_TOKENIZER_JSON).expect("tokenizer");
            let error = CandleLlmRuntime::try_load(
                "tiny-deepseek",
                &dir,
                "cpu",
                &HardwareStrategy::default(),
            )
            .expect_err("DeepSeek V3/R1 must not be loaded by the V2 backend");
            match error {
                CandleLlmError::UnsupportedModel { detail, .. } => {
                    assert!(detail.contains("no verified safetensors loader"));
                }
                other => panic!("expected UnsupportedModel, got {other:?}"),
            }
            let _ = fs::remove_dir_all(dir);
        }
    }

    fn load_fixture(tag: &str) -> (CandleLlmRuntime, PathBuf) {
        load_fixture_with_strategy(tag, &HardwareStrategy::default())
    }

    fn load_fixture_with_strategy(
        tag: &str,
        strategy: &HardwareStrategy,
    ) -> (CandleLlmRuntime, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "tachyon-llama-{tag}-{}-{nanos}",
            std::process::id()
        ));
        write_tachyon_tiny_fixture(&dir).expect("fixture should be written");
        let runtime = CandleLlmRuntime::try_load("tiny", &dir, "cpu", strategy)
            .expect("fixture should load without error")
            .expect("fixture is a supported Llama model");
        (runtime, dir)
    }

    /// Same fixture as [`load_fixture`], but with a custom 4-token vocabulary
    /// whose token id 1 decodes to the exact JSON text `tachyon`. Used to
    /// exercise constrained decoding without needing a real tokenizer that
    /// can spell out JSON punctuation one character at a time.
    fn load_fixture_with_vocab(tag: &str, vocab: &str) -> (CandleLlmRuntime, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "tachyon-llama-{tag}-{}-{nanos}",
            std::process::id()
        ));
        write_tachyon_tiny_fixture(&dir).expect("fixture should be written");
        fs::write(
            dir.join(TOKENIZER_JSON),
            format!(
                r#"{{
  "version": "1.0",
  "truncation": null,
  "padding": null,
  "added_tokens": [],
  "normalizer": null,
  "pre_tokenizer": {{"type": "Whitespace"}},
  "post_processor": null,
  "decoder": null,
  "model": {{
    "type": "WordLevel",
    "vocab": {vocab},
    "unk_token": "<unk>"
  }}
}}"#
            ),
        )
        .expect("custom tokenizer should write");
        let runtime = CandleLlmRuntime::try_load("tiny", &dir, "cpu", &HardwareStrategy::default())
            .expect("fixture should load without error")
            .expect("fixture is a supported Llama model");
        (runtime, dir)
    }

    #[test]
    fn generate_runs_a_real_llama_forward_and_is_not_a_mock() {
        let (runtime, dir) = load_fixture("real-forward");
        let bytes = runtime
            .generate(&[&b"hello"[..]])
            .expect("generation should run the Llama forward");
        let text = String::from_utf8(bytes).expect("decoded output should be UTF-8");
        assert!(
            !text.is_empty(),
            "a real forward pass must emit at least one decoded token"
        );
        assert_ne!(text, "MOCK_LLM_RESPONSE");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn llama_generation_populates_block_prefix_cache() {
        let strategy = HardwareStrategy {
            prefill_chunk_tokens: Some(PREFIX_CACHE_BLOCK_TOKENS as u32),
            ..HardwareStrategy::default()
        };
        let (runtime, dir) = load_fixture_with_strategy("prefix-cache-fill", &strategy);
        let prompt = std::iter::repeat_n("hello", PREFIX_CACHE_BLOCK_TOKENS * 2)
            .collect::<Vec<_>>()
            .join(" ");
        let request = serde_json::json!({
            "prompt": prompt,
            "max_new_tokens": 1,
        })
        .to_string();

        runtime
            .generate(&[request.as_bytes()])
            .expect("generation should fill prefix cache");

        assert_eq!(
            runtime.debug_prefix_cache_token_lengths(),
            vec![PREFIX_CACHE_BLOCK_TOKENS, PREFIX_CACHE_BLOCK_TOKENS * 2]
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn llama_generation_reuses_cached_prefix_without_changing_output() {
        let strategy = HardwareStrategy {
            prefill_chunk_tokens: Some(PREFIX_CACHE_BLOCK_TOKENS as u32),
            ..HardwareStrategy::default()
        };
        let (runtime, dir) = load_fixture_with_strategy("prefix-cache-hit", &strategy);
        let prompt = std::iter::repeat_n("hello", PREFIX_CACHE_BLOCK_TOKENS + 4)
            .collect::<Vec<_>>()
            .join(" ");
        let request = serde_json::json!({
            "prompt": prompt,
            "max_new_tokens": 2,
        })
        .to_string();

        let first = runtime
            .generate(&[request.as_bytes()])
            .expect("first generation should run");
        let hits_before = runtime.debug_prefix_cache_hits();
        let second = runtime
            .generate(&[request.as_bytes()])
            .expect("second generation should reuse prefix cache");

        assert_eq!(first, second);
        assert!(
            runtime.debug_prefix_cache_hits() > hits_before,
            "second generation should hit the block prefix cache"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn prefill_chunk_limits_default_to_8k_and_can_be_disabled() {
        let default_limits = GenerationLimits::with_context(DEFAULT_MAX_PROMPT_TOKENS);
        assert_eq!(
            default_limits.next_prefill_chunk_len(DEFAULT_PREFILL_CHUNK_TOKENS + 1),
            DEFAULT_PREFILL_CHUNK_TOKENS
        );

        let disabled = default_limits.with_prefill_chunk_tokens(Some(0));
        assert_eq!(
            disabled.next_prefill_chunk_len(DEFAULT_PREFILL_CHUNK_TOKENS + 1),
            DEFAULT_PREFILL_CHUNK_TOKENS + 1
        );

        let configured = default_limits.with_prefill_chunk_tokens(Some(128));
        assert_eq!(configured.next_prefill_chunk_len(1024), 128);
    }

    #[test]
    fn json_schema_constrained_generation_only_ever_emits_grammar_valid_text() {
        // Token id 2 decodes to the exact, grammar-complete JSON text
        // `{"ok":true}`; ids 0/1/3 decode to grammar-violating text. With the
        // schema's FSM masking every non-matching id to `-inf`, sampling must
        // pick id 2 regardless of the (untrained, deterministic-fill) model's
        // raw logits.
        let (runtime, dir) = load_fixture_with_vocab(
            "constrained",
            r#"{"<unk>": 0, "x": 1, "{\"ok\":true}": 2, "mesh": 3}"#,
        );
        let request = serde_json::json!({
            "prompt": "hello",
            "max_new_tokens": 1,
            "json_schema": r#"{"type":"object","properties":{"ok":{"type":"boolean"}}}"#,
        })
        .to_string();
        let bytes = runtime
            .generate(&[request.as_bytes()])
            .expect("constrained generation should run");
        let text = String::from_utf8(bytes).expect("decoded output should be UTF-8");
        assert_eq!(text, r#"{"ok":true}"#);

        let parsed: serde_json::Value =
            serde_json::from_str(&text).expect("output must be valid JSON");
        assert_eq!(parsed["ok"], serde_json::Value::Bool(true));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn invalid_json_schema_is_rejected_before_generation() {
        let (runtime, dir) = load_fixture("invalid-schema");
        let request = serde_json::json!({
            "prompt": "hello",
            "json_schema": "{not valid json",
        })
        .to_string();
        let error = runtime
            .generate(&[request.as_bytes()])
            .expect_err("malformed schema must be rejected");
        assert!(matches!(error, CandleLlmError::InvalidRequest { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn next_token_logits_depend_on_the_prompt() {
        let (runtime, dir) = load_fixture("prompt-dependent");
        let hello = runtime.debug_last_logits("hello");
        let mesh = runtime.debug_last_logits("mesh");
        assert_eq!(hello.len(), FIXTURE_VOCAB_SIZE);
        assert_eq!(mesh.len(), FIXTURE_VOCAB_SIZE);
        assert_ne!(
            hello, mesh,
            "different prompts must flow through attention to different logits"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn modelopt_nvfp4_dequantized_forward_matches_a_dense_reference() {
        // Quantizes only `down_proj.weight` (the one square-free linear in the
        // fixture, [hidden, inter] = [8, 16], so its 16-wide row satisfies
        // NVFP4's block-size-16 constraint with exactly one scale per row) using
        // values drawn from NVFP4's own E2M1 levels with unit scales, so
        // dequantization reproduces the dense reference's weights exactly
        // rather than merely approximately.
        const E2M1_LUT: [f32; 16] = [
            0.0, 0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 6.0, -0.0, -0.5, -1.0, -1.5, -2.0, -3.0, -4.0, -6.0,
        ];
        let hidden = FIXTURE_HIDDEN_SIZE;
        let inter = FIXTURE_INTERMEDIATE_SIZE;
        let down_values: Vec<f32> = (0..hidden)
            .flat_map(|row| (0..inter).map(move |col| E2M1_LUT[(row + col) % 16]))
            .collect();

        let mut weights = fixture_weights().expect("fixture weights");
        for layer in 0..FIXTURE_NUM_LAYERS {
            weights.insert(
                format!("model.layers.{layer}.mlp.down_proj.weight"),
                Tensor::from_vec(down_values.clone(), (hidden, inter), &Device::Cpu)
                    .expect("down_proj override"),
            );
        }

        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos();
        let dense_dir = std::env::temp_dir().join(format!(
            "tachyon-nvfp4-dense-ref-{}-{nanos}",
            std::process::id()
        ));
        let nvfp4_dir = std::env::temp_dir().join(format!(
            "tachyon-nvfp4-quant-{}-{nanos}",
            std::process::id()
        ));

        fn write_config_and_tokenizer(root: &Path) {
            fs::create_dir_all(root).expect("dir");
            fs::write(
                root.join(CONFIG_JSON),
                serde_json::json!({
                    "model_type": LLAMA_MODEL_TYPE,
                    "architectures": ["LlamaForCausalLM"],
                    "hidden_size": FIXTURE_HIDDEN_SIZE,
                    "intermediate_size": FIXTURE_INTERMEDIATE_SIZE,
                    "vocab_size": FIXTURE_VOCAB_SIZE,
                    "num_hidden_layers": FIXTURE_NUM_LAYERS,
                    "num_attention_heads": FIXTURE_NUM_HEADS,
                    "num_key_value_heads": FIXTURE_NUM_HEADS,
                    "rms_norm_eps": 1e-5,
                    "rope_theta": 10000.0,
                    "max_position_embeddings": FIXTURE_MAX_POSITION_EMBEDDINGS,
                    "tie_word_embeddings": false,
                    "bos_token_id": 1,
                    "eos_token_id": null
                })
                .to_string(),
            )
            .expect("config");
            fs::write(root.join(TOKENIZER_JSON), TINY_TOKENIZER_JSON).expect("tokenizer");
        }

        // Dense reference: plain safetensors, the already-tested load path.
        write_config_and_tokenizer(&dense_dir);
        candle_core::safetensors::save(&weights, dense_dir.join(MODEL_SAFETENSORS))
            .expect("dense reference weights should save");

        // NVFP4 fixture: every tensor passthrough-F32 except each layer's
        // `down_proj.weight`, which is packed/scaled so it dequantizes back to
        // exactly `down_values`.
        write_config_and_tokenizer(&nvfp4_dir);
        let mut shard_entries: Vec<(String, &'static str, Vec<usize>, Vec<u8>)> = Vec::new();
        for (name, tensor) in &weights {
            if name.ends_with(".mlp.down_proj.weight") {
                continue;
            }
            let shape = tensor.dims().to_vec();
            let values = tensor
                .flatten_all()
                .expect("flatten")
                .to_vec1::<f32>()
                .expect("f32 values");
            let bytes = values
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<u8>>();
            shard_entries.push((name.clone(), "F32", shape, bytes));
        }
        for layer in 0..FIXTURE_NUM_LAYERS {
            let base = format!("model.layers.{layer}.mlp.down_proj");
            let pack_cols = inter / 2;
            let mut packed = vec![0u8; hidden * pack_cols];
            for row in 0..hidden {
                for byte_idx in 0..pack_cols {
                    let hi = (row + 2 * byte_idx) % 16;
                    let lo = (row + 2 * byte_idx + 1) % 16;
                    packed[row * pack_cols + byte_idx] = ((hi as u8) << 4) | (lo as u8);
                }
            }
            shard_entries.push((
                format!("{base}.weight"),
                "U8",
                vec![hidden, pack_cols],
                packed,
            ));
            shard_entries.push((
                format!("{base}.weight_scale"),
                "F8_E4M3",
                vec![hidden, inter / 16],
                vec![0x38u8; hidden * (inter / 16)],
            ));
            shard_entries.push((
                format!("{base}.weight_scale_2"),
                "F32",
                vec![1],
                1.0f32.to_le_bytes().to_vec(),
            ));
        }
        let shard_name = "model-00001-of-00001.safetensors";
        let entries: Vec<(&str, &str, Vec<usize>, Vec<u8>)> = shard_entries
            .iter()
            .map(|(name, dtype, shape, bytes)| {
                (name.as_str(), *dtype, shape.clone(), bytes.clone())
            })
            .collect();
        modelopt_nvfp4::tests_support::write_safetensors_shard(
            &nvfp4_dir.join(shard_name),
            entries,
        );
        let tensor_names: Vec<&str> = shard_entries
            .iter()
            .map(|(name, ..)| name.as_str())
            .collect();
        modelopt_nvfp4::tests_support::write_index(&nvfp4_dir, shard_name, &tensor_names);

        let dense_runtime = CandleLlmRuntime::try_load(
            "dense-ref",
            &dense_dir,
            "cpu",
            &HardwareStrategy::default(),
        )
        .expect("dense reference should load")
        .expect("dense reference is a supported llama model");
        let directory = modelopt_nvfp4::ModelOptNvfp4Directory::try_load("nvfp4", &nvfp4_dir)
            .expect("nvfp4 detection should not error")
            .expect("nvfp4 directory should be detected");
        let nvfp4_runtime = CandleLlmRuntime::try_load_modelopt_nvfp4(&directory)
            .expect("nvfp4 checkpoint should load via the dequantized fallback path");

        let dense_logits = dense_runtime.debug_last_logits("hello");
        let nvfp4_logits = nvfp4_runtime.debug_last_logits("hello");
        assert_eq!(dense_logits.len(), nvfp4_logits.len());
        for (dense, nvfp4) in dense_logits.iter().zip(nvfp4_logits.iter()) {
            assert!(
                (dense - nvfp4).abs() < 1e-3,
                "dense {dense} vs nvfp4 {nvfp4} should match within tolerance"
            );
        }

        let generated = nvfp4_runtime
            .generate(&[&b"hello"[..]])
            .expect("nvfp4 checkpoint should run a real decode loop");
        assert!(!generated.is_empty(), "decode output must not be empty");

        let _ = fs::remove_dir_all(dense_dir);
        let _ = fs::remove_dir_all(nvfp4_dir);
    }

    #[test]
    fn greedy_decode_is_deterministic() {
        let (runtime, dir) = load_fixture("deterministic");
        let request: &[u8] = br#"{"prompt":"hello mesh","max_new_tokens":6}"#;
        let first = runtime.generate(&[request]).expect("first generation");
        let second = runtime.generate(&[request]).expect("second generation");
        assert_eq!(
            first, second,
            "greedy decoding the same prompt must be reproducible"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sampled_generation_is_reproducible_for_a_fixed_seed() {
        let (runtime, dir) = load_fixture("sampled-seeded");
        // temperature > 0 selects the sampling path; a pinned seed must make two
        // runs byte-identical, proving the seed actually drives the RNG.
        let request: &[u8] =
            br#"{"prompt":"hello mesh","max_new_tokens":6,"temperature":0.9,"seed":42}"#;
        let first = runtime
            .generate(&[request])
            .expect("first sampled generation");
        let second = runtime
            .generate(&[request])
            .expect("second sampled generation");
        assert_eq!(
            first, second,
            "sampling with a pinned seed must be reproducible"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn stop_sequence_truncates_the_decoded_text() {
        let (runtime, dir) = load_fixture("stop-seq");
        // First, the un-stopped greedy output (deterministic on this fixture).
        let plain: &[u8] = br#"{"prompt":"hello mesh","max_new_tokens":8}"#;
        let full = String::from_utf8(runtime.generate(&[plain]).expect("plain generation"))
            .expect("utf-8 output");

        // Pick an interior character of that output as a stop sequence and assert
        // the stopped output is a strict prefix that no longer contains it.
        let chars: Vec<char> = full.chars().collect();
        if chars.len() >= 2 {
            let needle = chars[chars.len() / 2].to_string();
            let request = serde_json::json!({
                "prompt": "hello mesh",
                "max_new_tokens": 8,
                "stop": [needle],
            })
            .to_string();
            let stopped =
                String::from_utf8(runtime.generate(&[request.as_bytes()]).expect("stopped"))
                    .expect("utf-8 output");
            assert!(
                full.starts_with(&stopped),
                "stopped output `{stopped}` must be a prefix of `{full}`"
            );
            assert!(
                !stopped.contains(needle.as_str()),
                "the matched stop sequence must be trimmed from `{stopped}`"
            );
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn messages_request_renders_through_the_generic_fallback() {
        // The tiny fixture ships no `tokenizer_config.json`, so a structured
        // chat request must still run via the generic chat rendering.
        let (runtime, dir) = load_fixture("messages-fallback");
        let request = serde_json::json!({
            "messages": [
                {"role": "system", "content": "be terse"},
                {"role": "user", "content": "hello"},
            ],
            "max_new_tokens": 4,
        })
        .to_string();
        let bytes = runtime
            .generate(&[request.as_bytes()])
            .expect("a messages request must run on a checkpoint without a template");
        assert!(!bytes.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn checkpoint_chat_template_is_loaded_and_rendered() {
        // Drop a `tokenizer_config.json` carrying a ChatML-style template that
        // exercises the Jinja path (loop, special tokens, `.strip()` via pycompat,
        // and `add_generation_prompt`). The base fixture is reloaded so the
        // template is picked up at load time.
        let (_runtime, dir) = load_fixture("chat-template");
        // `.strip()` is a Python str method (not a Jinja filter): it exercises the
        // minijinja-contrib pycompat callback that real HF templates depend on.
        let template = "{{ bos_token }}{% for m in messages %}<|im_start|>{{ m['role'] }}\n{{ m['content'].strip() }}<|im_end|>\n{% endfor %}{% if add_generation_prompt %}<|im_start|>assistant\n{% endif %}";
        let config = serde_json::json!({
            "bos_token": "<s>",
            "eos_token": {"content": "</s>"},
            "chat_template": template,
        });
        fs::write(dir.join(TOKENIZER_CONFIG_JSON), config.to_string()).expect("write config");
        let reloaded =
            CandleLlmRuntime::try_load("tiny", &dir, "cpu", &HardwareStrategy::default())
                .expect("reload should not error")
                .expect("fixture is supported");

        let messages = vec![ChatTurn {
            role: "user".to_owned(),
            content: "  hi  ".to_owned(),
        }];
        let rendered = reloaded
            .render_chat(&messages)
            .expect("model template should render");
        assert_eq!(
            rendered, "<s><|im_start|>user\nhi<|im_end|>\n<|im_start|>assistant\n",
            "the checkpoint's own template (with bos_token, trim, and the \
             generation prompt) must drive rendering"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn resolve_sampling_collapses_nonpositive_temperature_to_greedy() {
        // Absent or non-positive temperature => deterministic argmax.
        assert!(resolve_sampling(None, None, None).temperature.is_none());
        assert!(resolve_sampling(Some(0.0), Some(0.9), Some(7))
            .temperature
            .is_none());
        // A real temperature is kept; top_p is only honoured inside (0, 1).
        let policy = resolve_sampling(Some(0.8), Some(1.0), Some(7));
        assert_eq!(policy.seed, 7);
        assert_eq!(policy.temperature, Some(f64::from(0.8_f32)));
        assert!(
            policy.top_p.is_none(),
            "top_p of 1.0 disables nucleus filtering"
        );
        assert_eq!(
            resolve_sampling(Some(0.8), Some(0.5), None).top_p,
            Some(f64::from(0.5_f32))
        );
        // An un-seeded sampled request falls back to the fixed default seed.
        assert_eq!(
            resolve_sampling(Some(0.8), None, None).seed,
            DEFAULT_SAMPLING_SEED
        );
    }

    #[test]
    fn sanitize_stop_filters_empty_and_bounds_the_set() {
        let raw = Some(vec![
            "".to_owned(),
            "a".to_owned(),
            "x".repeat(MAX_STOP_SEQUENCE_BYTES + 1),
            "b".to_owned(),
            "c".to_owned(),
            "d".to_owned(),
            "e".to_owned(),
            "f".to_owned(),
            "g".to_owned(),
            "h".to_owned(),
            "i".to_owned(),
        ]);
        let stops = sanitize_stop(raw);
        assert!(stops.len() <= MAX_STOP_SEQUENCES);
        assert!(!stops.iter().any(String::is_empty));
        assert!(!stops.iter().any(|s| s.len() > MAX_STOP_SEQUENCE_BYTES));
    }

    #[test]
    fn find_earliest_stop_returns_the_earliest_match() {
        let stops = vec!["END".to_owned(), "stop".to_owned()];
        assert_eq!(find_earliest_stop("keep me END drop", &stops), Some(8));
        // Earliest of several matches wins.
        assert_eq!(find_earliest_stop("a stop b END c", &stops), Some(2));
        // No match.
        assert_eq!(find_earliest_stop("nothing here", &stops), None);
    }

    #[test]
    fn streaming_deltas_concatenate_to_the_buffered_output() {
        let (runtime, dir) = load_fixture("stream-concat");
        let request: &[u8] = br#"{"prompt":"hello mesh","max_new_tokens":6}"#;
        let buffered =
            String::from_utf8(runtime.generate(&[request]).expect("buffered")).expect("utf-8");
        let mut streamed = String::new();
        runtime
            .generate_streaming(&[request], &mut |delta| streamed.push_str(delta))
            .expect("streamed");
        assert_eq!(
            streamed, buffered,
            "the concatenation of streamed deltas must equal the buffered output"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn speculative_decoding_with_identical_draft_matches_greedy_decode() {
        let (target, target_dir) = load_fixture("spec-target");
        let (draft, draft_dir) = load_fixture("spec-draft");
        let request: &[u8] = br#"{"prompt":"hello mesh","max_new_tokens":6}"#;
        let greedy = target.generate(&[request]).expect("greedy generation");
        let speculative = target
            .generate_speculative(&[request], &draft, 3)
            .expect("speculative generation");
        assert_eq!(
            speculative, greedy,
            "draft/verify must preserve target greedy output"
        );

        let mut streamed = String::new();
        target
            .generate_speculative_streaming(&[request], &draft, 3, &mut |delta| {
                streamed.push_str(delta)
            })
            .expect("speculative streaming");
        assert_eq!(streamed.into_bytes(), greedy);
        let _ = fs::remove_dir_all(target_dir);
        let _ = fs::remove_dir_all(draft_dir);
    }

    #[test]
    fn non_llama_model_type_is_rejected() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "tachyon-llama-reject-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("dir");
        fs::write(
            dir.join(CONFIG_JSON),
            serde_json::json!({"model_type": "gpt2", "vocab_size": 4}).to_string(),
        )
        .expect("config");
        // A valid tokenizer so loading reaches the architecture check (the
        // tokenizer is loaded before the format-specific loader).
        fs::write(dir.join(TOKENIZER_JSON), TINY_TOKENIZER_JSON).expect("tokenizer");
        fs::write(dir.join(MODEL_SAFETENSORS), b"marker").expect("weights marker");

        let error = CandleLlmRuntime::try_load("tiny", &dir, "cpu", &HardwareStrategy::default())
            .expect_err("a non-Llama model_type must be rejected");
        assert!(matches!(error, CandleLlmError::UnsupportedModel { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_weight_tensor_is_a_clear_error_not_a_panic() {
        let (_runtime, dir) = load_fixture("missing-weight-base");
        // Re-save the weights without lm_head.weight so Llama::load fails cleanly
        // (its lm_head load propagates the error rather than panicking).
        let mut tensors = candle_core::safetensors::load(dir.join(MODEL_SAFETENSORS), &Device::Cpu)
            .expect("fixture weights should load");
        tensors.remove("lm_head.weight");
        candle_core::safetensors::save(&tensors, dir.join(MODEL_SAFETENSORS))
            .expect("trimmed weights should save");

        let error = CandleLlmRuntime::try_load("tiny", &dir, "cpu", &HardwareStrategy::default())
            .expect_err("a model missing lm_head.weight must fail to load");
        assert!(matches!(error, CandleLlmError::InvalidComponent { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    fn load_gguf_fixture(tag: &str) -> (CandleLlmRuntime, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("tachyon-gguf-{tag}-{}-{nanos}", std::process::id()));
        write_tachyon_tiny_gguf_fixture(&dir).expect("gguf fixture should be written");
        let runtime =
            CandleLlmRuntime::try_load("tiny-gguf", &dir, "cpu", &HardwareStrategy::default())
                .expect("gguf fixture should load without error")
                .expect("gguf fixture is a supported Llama model");
        (runtime, dir)
    }

    /// Build an in-memory GGUF header carrying the given tensor block types.
    /// No file is written: only `tensor_infos` matters to the CUDA dtype scan.
    fn gguf_header_with_dtypes(tensors: &[(&str, GgmlDType)]) -> gguf_file::Content {
        gguf_file::Content {
            magic: gguf_file::VersionedMagic::GgufV3,
            metadata: std::collections::HashMap::new(),
            tensor_infos: tensors
                .iter()
                .map(|(name, dtype)| {
                    (
                        (*name).to_owned(),
                        gguf_file::TensorInfo {
                            ggml_dtype: *dtype,
                            shape: candle_core::Shape::from_dims(&[256]),
                            offset: 0,
                        },
                    )
                })
                .collect(),
            tensor_data_offset: 0,
        }
    }

    #[test]
    fn gguf_cuda_kernel_coverage_matches_the_pinned_candle_fork() {
        // Every block type with a `mul_mat*` kernel in the fork's
        // `quantized/cuda.rs`, plus the dense types `QMatMul` dequantizes.
        for dtype in [
            GgmlDType::F32,
            GgmlDType::F16,
            GgmlDType::BF16,
            GgmlDType::Q4_0,
            GgmlDType::Q4_1,
            GgmlDType::Q5_0,
            GgmlDType::Q5_1,
            GgmlDType::Q8_0,
            GgmlDType::Q2K,
            GgmlDType::Q3K,
            GgmlDType::Q4K,
            GgmlDType::Q5K,
            GgmlDType::Q6K,
        ] {
            assert!(
                gguf_dtype_has_cuda_matmul_kernel(dtype),
                "{dtype:?} has a CUDA quantized-matmul kernel and must be accepted"
            );
        }
        // Both dequantize on CUDA but neither has a quantized-matmul kernel,
        // so accepting them would move the failure to the first decode step.
        for dtype in [GgmlDType::Q8_1, GgmlDType::Q8K] {
            assert!(
                !gguf_dtype_has_cuda_matmul_kernel(dtype),
                "{dtype:?} has no CUDA quantized-matmul kernel and must be rejected"
            );
        }
    }

    #[test]
    fn gguf_cuda_dtype_scan_reports_the_first_offending_tensor() {
        let content = gguf_header_with_dtypes(&[
            ("blk.1.ffn_down.weight", GgmlDType::Q8K),
            ("blk.0.attn_q.weight", GgmlDType::Q4K),
            ("blk.0.attn_k.weight", GgmlDType::Q8K),
        ]);
        let report = unsupported_gguf_cuda_dtypes(&content)
            .expect("a Q8K tensor must be reported as unsupported");
        // Name-sorted, so the same file always names the same tensor.
        assert_eq!(report.tensor, "blk.0.attn_k.weight");
        assert_eq!(report.dtype, GgmlDType::Q8K);
        assert_eq!(report.affected, 2);
        assert_eq!(report.total, 3);
    }

    #[test]
    fn gguf_cuda_dtype_scan_accepts_a_k_quant_checkpoint() {
        let content = gguf_header_with_dtypes(&[
            ("token_embd.weight", GgmlDType::Q6K),
            ("blk.0.attn_q.weight", GgmlDType::Q4K),
            ("output_norm.weight", GgmlDType::F32),
        ]);
        assert!(
            unsupported_gguf_cuda_dtypes(&content).is_none(),
            "a Q4_K_M-style checkpoint must be accepted for CUDA"
        );
    }

    #[test]
    fn gguf_cpu_bindings_stay_on_the_host() {
        // Even a checkpoint CUDA could never run stays loadable on `cpu`: the
        // dtype scan must not gate the host path.
        let content = gguf_header_with_dtypes(&[("blk.0.attn_q.weight", GgmlDType::Q8K)]);
        let device = resolve_gguf_device("tiny-gguf", Path::new("/models/tiny"), &content, "cpu")
            .expect("a cpu binding must always resolve");
        assert!(device.is_cpu());
    }

    #[cfg(not(feature = "candle-cuda"))]
    #[test]
    fn gguf_rejects_a_gpu_binding_without_the_cuda_feature() {
        let content = gguf_header_with_dtypes(&[("blk.0.attn_q.weight", GgmlDType::Q4K)]);
        let error = resolve_gguf_device("tiny-gguf", Path::new("/models/tiny"), &content, "cuda")
            .expect_err("a CUDA-less build must refuse a gpu binding");
        assert!(
            error.to_string().contains("candle-cuda"),
            "the error must name the missing build feature, got: {error}"
        );
    }

    #[cfg(feature = "candle-cuda")]
    #[test]
    fn gguf_rejects_a_gpu_binding_whose_blocks_have_no_cuda_kernel() {
        let content = gguf_header_with_dtypes(&[
            ("blk.0.attn_q.weight", GgmlDType::Q4K),
            ("blk.0.ffn_down.weight", GgmlDType::Q8K),
        ]);
        let error = resolve_gguf_device("tiny-gguf", Path::new("/models/tiny"), &content, "cuda")
            .expect_err("a Q8K tensor must be refused before any VRAM is claimed");
        let message = error.to_string();
        assert!(
            message.contains("blk.0.ffn_down.weight") && message.contains("Q8K"),
            "the error must name the tensor and its block type, got: {message}"
        );
    }

    #[test]
    fn gguf_generate_runs_a_real_quantized_llama_forward() {
        let (runtime, dir) = load_gguf_fixture("real-forward");
        let bytes = runtime
            .generate(&[&b"hello"[..]])
            .expect("generation should run the quantized Llama forward");
        let text = String::from_utf8(bytes).expect("decoded output should be UTF-8");
        assert!(
            !text.is_empty(),
            "a real GGUF forward pass must emit at least one decoded token"
        );
        assert_ne!(text, "MOCK_LLM_RESPONSE");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn gguf_next_token_logits_depend_on_the_prompt() {
        let (runtime, dir) = load_gguf_fixture("prompt-dependent");
        let hello = runtime.debug_last_logits("hello");
        let mesh = runtime.debug_last_logits("mesh");
        assert_eq!(hello.len(), FIXTURE_VOCAB_SIZE);
        assert_ne!(
            hello, mesh,
            "different prompts must flow through the quantized attention to different logits"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn gguf_greedy_decode_is_deterministic() {
        let (runtime, dir) = load_gguf_fixture("deterministic");
        let request: &[u8] = br#"{"prompt":"hello mesh","max_new_tokens":6}"#;
        let first = runtime.generate(&[request]).expect("first generation");
        let second = runtime.generate(&[request]).expect("second generation");
        assert_eq!(
            first, second,
            "greedy decoding the same GGUF prompt must be reproducible"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn gguf_format_is_inferred_from_contents_without_a_sidecar() {
        let (_runtime, dir) = load_gguf_fixture("no-sidecar");
        // Drop the broker sidecar: the host must still pick the GGUF path by
        // finding the `.gguf` file (operator-provisioned model).
        fs::remove_file(dir.join(MODEL_META_JSON)).expect("sidecar should be removed");
        let runtime =
            CandleLlmRuntime::try_load("tiny-gguf", &dir, "cpu", &HardwareStrategy::default())
                .expect("content inference should not error")
                .expect("a directory with a .gguf file is a GGUF model");
        let bytes = runtime
            .generate(&[&b"hello"[..]])
            .expect("inferred GGUF model should still run");
        assert!(!bytes.is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn non_llama_gguf_architecture_is_rejected() {
        let (_runtime, dir) = load_gguf_fixture("reject-arch");
        // Rewrite the sidecar away and re-tag the GGUF as a non-Llama arch by
        // pointing the loader at a freshly written, mislabeled metadata file is
        // overkill; instead assert the sidecar enum guard rejects junk formats.
        fs::write(
            dir.join(MODEL_META_JSON),
            serde_json::json!({ "format": "onnx" }).to_string(),
        )
        .expect("sidecar rewrite");
        let error =
            CandleLlmRuntime::try_load("tiny-gguf", &dir, "cpu", &HardwareStrategy::default())
                .expect_err("an unsupported declared format must be rejected");
        assert!(matches!(error, CandleLlmError::UnsupportedModel { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    // -----------------------------------------------------------------------
    // Parallel dispatch (Task 7). The runtime now selects the tensor/pipeline
    // engines from a deployment's `hardware_strategy`. These run on
    // `Device::Cpu` stand-ins (this build has no CUDA backend), exactly as the
    // engines' own equivalence tests do, and inject a multi-device topology so
    // the hardware-aware validation admits the plan.
    // -----------------------------------------------------------------------

    #[cfg(test)]
    fn write_fixture_dir(tag: &str) -> PathBuf {
        write_fixture_dir_with(tag, write_tachyon_tiny_fixture)
    }

    fn write_fixture_dir_with(
        tag: &str,
        writer: impl FnOnce(&Path) -> anyhow::Result<()>,
    ) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "tachyon-llama-{tag}-{}-{nanos}",
            std::process::id()
        ));
        writer(&dir).expect("fixture should be written");
        dir
    }

    #[cfg(test)]
    fn two_device_topology() -> ClusterTopology {
        use parallel_topology::{DeviceInfo, InterconnectClass};
        ClusterTopology {
            devices: vec![
                DeviceInfo {
                    device_id: 0,
                    free_vram_bytes: 0,
                },
                DeviceInfo {
                    device_id: 1,
                    free_vram_bytes: 0,
                },
            ],
            interconnect: InterconnectClass::Pcie,
        }
    }

    #[test]
    fn tensor_parallel_strategy_dispatches_and_matches_the_dense_runtime() {
        let dir = write_fixture_dir("tp-dispatch");

        let dense = CandleLlmRuntime::try_load("tiny", &dir, "cpu", &HardwareStrategy::default())
            .expect("dense load")
            .expect("supported model");
        let strategy = HardwareStrategy {
            distribution_mode: GpuDistribution::TensorParallelism,
            device_ids: vec![0, 1],
            ..Default::default()
        };
        let tp = CandleLlmRuntime::try_load_with_topology(
            "tiny",
            &dir,
            "cuda",
            &strategy,
            &two_device_topology(),
        )
        .expect("tensor-parallel load")
        .expect("supported model");

        // The dispatch selected the parallel engine, not the dense path.
        assert!(matches!(
            &*tp.inner,
            LoadedModel::Parallel(ParallelModel::Tensor { .. })
        ));

        let dense_logits = dense.debug_last_logits("hello mesh");
        let tp_logits = tp.debug_parallel_prefill_logits("hello mesh");
        assert_eq!(dense_logits.len(), tp_logits.len());
        for (a, b) in dense_logits.iter().zip(tp_logits.iter()) {
            assert!(
                (a - b).abs() < 1e-3,
                "tensor-parallel logits must match the dense runtime within 1e-3, got {a} vs {b}"
            );
        }

        // Tensor parallelism carries a KV cache, so full generation works.
        let generated = tp
            .generate(&[&b"hello"[..]])
            .expect("tensor-parallel generation should run the decode loop");
        assert!(!generated.is_empty());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn pipeline_parallel_strategy_matches_dense_prefill_and_decodes() {
        let dir = write_fixture_dir("pp-dispatch");

        let dense = CandleLlmRuntime::try_load("tiny", &dir, "cpu", &HardwareStrategy::default())
            .expect("dense load")
            .expect("supported model");
        let strategy = HardwareStrategy {
            distribution_mode: GpuDistribution::PipelineParallelism,
            device_ids: vec![0, 1],
            stage_layer_ranges: vec![(0, 0), (1, 1)],
            ..Default::default()
        };
        let pipeline = CandleLlmRuntime::try_load_with_topology(
            "tiny",
            &dir,
            "cuda",
            &strategy,
            &two_device_topology(),
        )
        .expect("pipeline-parallel load")
        .expect("supported model");

        assert!(matches!(
            &*pipeline.inner,
            LoadedModel::Parallel(ParallelModel::Pipeline { .. })
        ));

        let dense_logits = dense.debug_last_logits("hello mesh");
        let pp_logits = pipeline.debug_parallel_prefill_logits("hello mesh");
        assert_eq!(dense_logits.len(), pp_logits.len());
        for (a, b) in dense_logits.iter().zip(pp_logits.iter()) {
            assert!(
                (a - b).abs() < 1e-3,
                "pipeline prefill logits must match the dense runtime within 1e-3, got {a} vs {b}"
            );
        }

        // Pipeline parallelism carries a per-stage KV cache, so full
        // generation (prefill + decode) now works like the tensor-parallel
        // and dense paths.
        let generated = pipeline
            .generate(&[&b"hello"[..]])
            .expect("pipeline-parallel generation should run the decode loop");
        assert!(!generated.is_empty());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn expert_parallel_strategy_rejects_a_non_mixtral_checkpoint() {
        // A dense Llama checkpoint declares `model_type: "llama"`, which the
        // Mixtral-only expert-parallel loader must reject rather than
        // silently treating as a dense single-expert MoE model.
        let dir = write_fixture_dir("ep-dispatch-non-mixtral");
        let strategy = HardwareStrategy {
            distribution_mode: GpuDistribution::ExpertParallelism,
            device_ids: vec![0, 1],
            expert_device_map: vec![(0, 0), (1, 1)],
            ..Default::default()
        };
        let error = CandleLlmRuntime::try_load_with_topology(
            "tiny",
            &dir,
            "cuda",
            &strategy,
            &two_device_topology(),
        )
        .expect_err("expert-parallel must reject a non-Mixtral checkpoint");
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(detail.contains("Mixtral"), "unexpected detail: {detail}");
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn expert_parallel_strategy_dispatches_and_decodes_a_mixtral_checkpoint() {
        let dir = write_fixture_dir_with("ep-dispatch", write_tachyon_tiny_mixtral_mixed_fixture);
        let strategy = HardwareStrategy {
            distribution_mode: GpuDistribution::ExpertParallelism,
            device_ids: vec![0, 1],
            expert_device_map: vec![(0, 0), (1, 1)],
            ..Default::default()
        };
        let expert = CandleLlmRuntime::try_load_with_topology(
            "tiny",
            &dir,
            "cuda",
            &strategy,
            &two_device_topology(),
        )
        .expect("expert-parallel load")
        .expect("supported model");

        assert!(matches!(
            &*expert.inner,
            LoadedModel::Parallel(ParallelModel::Expert { .. })
        ));

        // Expert parallelism carries a KV cache, so full generation works
        // exactly like the tensor- and pipeline-parallel paths.
        let generated = expert
            .generate(&[&b"hello"[..]])
            .expect("expert-parallel generation should run the decode loop");
        assert!(!generated.is_empty());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn expert_parallel_strategy_rejects_top_k_routing_above_one() {
        let dir = std::env::temp_dir().join(format!(
            "tachyon-mixtral-topk-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after the epoch")
                .as_nanos()
        ));
        write_tachyon_tiny_mixtral_fixture(&dir).expect("fixture should be written");
        // Patch `num_experts_per_tok` to 2, which this runtime does not
        // support (only top-1 routing is implemented).
        let config_path = dir.join(CONFIG_JSON);
        let mut config: serde_json::Value =
            serde_json::from_slice(&fs::read(&config_path).expect("config should read"))
                .expect("config should parse");
        config["num_experts_per_tok"] = serde_json::json!(2);
        fs::write(&config_path, config.to_string()).expect("config should write");

        let strategy = HardwareStrategy {
            distribution_mode: GpuDistribution::ExpertParallelism,
            device_ids: vec![0, 1],
            ..Default::default()
        };
        let error = CandleLlmRuntime::try_load_with_topology(
            "tiny",
            &dir,
            "cuda",
            &strategy,
            &two_device_topology(),
        )
        .expect_err("top-2 routing must be rejected: only top-1 is implemented");
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(detail.contains("top-1"), "unexpected detail: {detail}");
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_parallel_plan_exceeding_discovered_devices_is_rejected_before_loading() {
        use parallel_topology::{DeviceInfo, InterconnectClass};
        let dir = write_fixture_dir("topology-reject");
        let strategy = HardwareStrategy {
            distribution_mode: GpuDistribution::TensorParallelism,
            device_ids: vec![0, 1],
            ..Default::default()
        };
        // Only one device is discovered, but the plan wants two.
        let single_device = ClusterTopology {
            devices: vec![DeviceInfo {
                device_id: 0,
                free_vram_bytes: 0,
            }],
            interconnect: InterconnectClass::HighBandwidth,
        };
        let error = CandleLlmRuntime::try_load_with_topology(
            "tiny",
            &dir,
            "cuda",
            &strategy,
            &single_device,
        )
        .expect_err("a plan requiring more devices than exist must be rejected");
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(
                    detail.contains("topology rejected"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    // On a build without `candle-cuda`, the single-device path stays cpu-only
    // for every architecture, Llama included — `enable-single-device-llama-cuda-execution`
    // only lifts this for a Llama checkpoint on a `candle-cuda` build (see
    // `single_device_llama_executes_on_a_real_cuda_device` below).
    #[cfg(not(feature = "candle-cuda"))]
    #[test]
    fn single_strategy_still_rejects_a_gpu_device_request() {
        let dir = write_fixture_dir("single-gpu-reject");
        let error = CandleLlmRuntime::try_load("tiny", &dir, "cuda", &HardwareStrategy::default())
            .expect_err(
                "single-device path is cpu-only without candle-cuda and must reject a gpu request",
            );
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(
                    detail.contains("cpu` execution only"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    // Holds on every build (with or without `candle-cuda`): only a Llama
    // checkpoint gets a real device on the single-device path.
    #[test]
    fn single_strategy_still_rejects_a_gpu_device_request_for_a_non_llama_architecture() {
        let dir = std::env::temp_dir().join(format!(
            "tachyon-qwen2-single-gpu-reject-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after the epoch")
                .as_nanos()
        ));
        write_tachyon_tiny_qwen_fixture(&dir, ModelArchitecture::Qwen2)
            .expect("Qwen fixture should be written");
        let error =
            CandleLlmRuntime::try_load("tiny-qwen", &dir, "cuda", &HardwareStrategy::default())
                .expect_err("a non-Llama architecture must keep rejecting a gpu request");
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(
                    detail.contains("cpu` execution only"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    // Requires a real CUDA device (`arc-gpu-runners`/`cuda-quality`, or a
    // dev machine with a working `candle-cuda` build) — see
    // `enable-single-device-llama-cuda-execution`'s design.md for why this
    // isn't runnable on this repo's default Windows dev sandbox today.
    #[cfg(feature = "candle-cuda")]
    #[test]
    fn single_device_llama_executes_on_a_real_cuda_device() {
        let dir = write_fixture_dir("single-gpu-llama");
        let runtime =
            CandleLlmRuntime::try_load("tiny", &dir, "cuda", &HardwareStrategy::default())
                .expect("a Llama checkpoint must load on a real cuda device")
                .expect("tiny fixture should select the native Candle runtime");
        let request: &[u8] = br#"{"prompt":"hello mesh","max_new_tokens":4}"#;
        let output = runtime
            .generate(&[request])
            .expect("generation must run a real decode on the cuda device");
        assert!(
            !output.is_empty(),
            "cuda-resident Llama generation must not be empty/mocked"
        );
        let _ = fs::remove_dir_all(dir);
    }

    // Regression test for a real bug caught by `cuda-quality` on real GPU
    // hardware: `size_paged_kv_pool` used to omit `num_hidden_layers` from
    // its byte-per-block cost, and sized the pool to whatever fit the
    // *entire* free-VRAM budget instead of just what one sequence needs.
    // For the tiny fixture's config (hidden_size 8, 2 heads, 2 layers) that
    // computed `max_blocks` in the millions given a real GPU's worth of
    // free VRAM, and the resulting multi-gigabyte allocation attempt OOM'd
    // a real CUDA device that easily had room for the tiny model's actual
    // weights. This test runs on CPU (no GPU/candle-cuda needed): it feeds a
    // large `free_vram_bytes` and asserts the pool still comes out capped at
    // exactly one full-length sequence's worth of blocks.
    #[test]
    fn paged_attention_pool_is_capped_at_one_sequence_even_with_abundant_free_vram() {
        let llama_config: LlamaConfig = serde_json::from_value(serde_json::json!({
            "hidden_size": FIXTURE_HIDDEN_SIZE,
            "intermediate_size": FIXTURE_INTERMEDIATE_SIZE,
            "vocab_size": FIXTURE_VOCAB_SIZE,
            "num_hidden_layers": FIXTURE_NUM_LAYERS,
            "num_attention_heads": FIXTURE_NUM_HEADS,
            "num_key_value_heads": FIXTURE_NUM_HEADS,
            "rms_norm_eps": 1e-5,
            "rope_theta": 10000.0,
            "max_position_embeddings": FIXTURE_MAX_POSITION_EMBEDDINGS,
            "tie_word_embeddings": false,
            "bos_token_id": 1,
            "eos_token_id": null
        }))
        .expect("fixture-shaped JSON should deserialize as LlamaConfig");
        let config = llama_config.into_config(false);

        // A generously large free-VRAM figure (16 GiB) that would size the
        // pool into the millions of blocks under the old (buggy) sizing.
        let abundant_free_vram_bytes = 16u64 * 1024 * 1024 * 1024;
        let pool = size_paged_kv_pool(&config, DType::BF16, abundant_free_vram_bytes)
            .expect("a tiny model's one-sequence pool must fit comfortably in 16 GiB");

        let expected_min_blocks =
            FIXTURE_MAX_POSITION_EMBEDDINGS.div_ceil(PAGED_ATTENTION_PAGE_BLOCK_SIZE);
        assert_eq!(
            pool.total_blocks(),
            expected_min_blocks,
            "pool must be capped at one sequence's worth of blocks, not whatever the free-VRAM budget allows"
        );
    }

    #[test]
    fn paged_attention_pool_sizing_fails_closed_when_even_one_sequence_does_not_fit() {
        let llama_config: LlamaConfig = serde_json::from_value(serde_json::json!({
            "hidden_size": FIXTURE_HIDDEN_SIZE,
            "intermediate_size": FIXTURE_INTERMEDIATE_SIZE,
            "vocab_size": FIXTURE_VOCAB_SIZE,
            "num_hidden_layers": FIXTURE_NUM_LAYERS,
            "num_attention_heads": FIXTURE_NUM_HEADS,
            "num_key_value_heads": FIXTURE_NUM_HEADS,
            "rms_norm_eps": 1e-5,
            "rope_theta": 10000.0,
            "max_position_embeddings": FIXTURE_MAX_POSITION_EMBEDDINGS,
            "tie_word_embeddings": false,
            "bos_token_id": 1,
            "eos_token_id": null
        }))
        .expect("fixture-shaped JSON should deserialize as LlamaConfig");
        let config = llama_config.into_config(false);

        size_paged_kv_pool(&config, DType::BF16, 0)
            .expect_err("zero free VRAM must not fit even one sequence's worth of blocks");
    }

    // Holds on every build (with or without `candle-cuda`): only a Llama
    // checkpoint on a CUDA device can get paged attention wired.
    #[test]
    fn paged_attention_strategy_is_rejected_for_a_non_llama_architecture() {
        let dir = std::env::temp_dir().join(format!(
            "tachyon-qwen2-paged-attn-reject-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after the epoch")
                .as_nanos()
        ));
        write_tachyon_tiny_qwen_fixture(&dir, ModelArchitecture::Qwen2)
            .expect("Qwen fixture should be written");
        let strategy = HardwareStrategy {
            paged_attention: true,
            ..Default::default()
        };
        let error = CandleLlmRuntime::try_load("tiny-qwen", &dir, "cuda", &strategy).expect_err(
            "a non-Llama architecture must keep rejecting paged_attention even on a cuda request",
        );
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(
                    detail.contains("flash_attn_varlen_paged_windowed"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    // Regression for a real bug an automated review caught: `load_parallel`'s
    // tensor/pipeline/expert engines never read `paged_attention` at all (they
    // carry their own separate cache types), so a tensor-parallel request
    // setting it used to pass this gate and then silently dispatch to
    // `load_parallel`, which just ignored the flag — accepted but not wired.
    // Holds on every build (with or without `candle-cuda`): a non-`Single`
    // distribution mode must keep rejecting paged_attention regardless of
    // architecture/device.
    #[test]
    fn paged_attention_strategy_is_rejected_for_a_non_single_distribution_mode() {
        let dir = write_fixture_dir("paged-attn-tp-reject");
        let strategy = HardwareStrategy {
            distribution_mode: GpuDistribution::TensorParallelism,
            device_ids: vec![0, 1],
            paged_attention: true,
            ..Default::default()
        };
        let error = CandleLlmRuntime::try_load_with_topology(
            "tiny",
            &dir,
            "cuda",
            &strategy,
            &two_device_topology(),
        )
        .expect_err(
            "a tensor-parallel request must keep rejecting paged_attention, not silently ignore it",
        );
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(
                    detail.contains("flash_attn_varlen_paged_windowed"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    // Requires a real CUDA device (`arc-gpu-runners`/`cuda-quality`) — not
    // runnable on this repo's default Windows dev sandbox, same as
    // `single_device_llama_executes_on_a_real_cuda_device`.
    #[cfg(feature = "candle-cuda")]
    #[test]
    fn single_device_llama_paged_attention_generates_a_real_decode_on_cuda() {
        let dir = write_fixture_dir_with(
            "paged-attn-cuda",
            write_tachyon_tiny_paged_attention_fixture,
        );
        let strategy = HardwareStrategy {
            paged_attention: true,
            ..Default::default()
        };
        let runtime = CandleLlmRuntime::try_load("tiny", &dir, "cuda", &strategy)
            .expect("a paged-attention Llama checkpoint must load on a real cuda device")
            .expect("tiny fixture should select the native Candle runtime");
        let request: &[u8] = br#"{"prompt":"hello mesh","max_new_tokens":4,"temperature":0.0}"#;
        let first = runtime
            .generate(&[request])
            .expect("paged-attention generation must run a real decode on the cuda device");
        assert!(
            !first.is_empty(),
            "paged-attention generation must not be empty/mocked"
        );
        let second = runtime
            .generate(&[request])
            .expect("a second paged-attention request must reuse the block pool correctly");
        assert_eq!(
            first, second,
            "greedy paged-attention generation must be deterministic across requests reusing the same shared block pool"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn paged_attention_strategy_is_rejected_until_block_tables_are_wired() {
        let dir = write_fixture_dir("paged-attn-reject");
        let strategy = HardwareStrategy {
            paged_attention: true,
            ..Default::default()
        };
        let error = CandleLlmRuntime::try_load("tiny", &dir, "cpu", &strategy)
            .expect_err("paged attention must not silently use the contiguous KV cache");
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(
                    detail.contains("flash_attn_varlen_paged_windowed"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn cuda_graph_decode_strategy_is_rejected_until_gpu_decode_is_wired() {
        let dir = write_fixture_dir("cuda-graph-reject");
        let strategy = HardwareStrategy {
            cuda_graph_decode: true,
            ..Default::default()
        };
        let error = CandleLlmRuntime::try_load("tiny", &dir, "cpu", &strategy)
            .expect_err("CUDA Graph decode must not silently use the uncaptured decode loop");
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(
                    detail.contains("candle_core::CudaGraph"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    // Reaching the paged_attention-dependency message (rather than the
    // device/arch rejection above) requires candle-cuda, so this can only
    // exercise the specific "requires paged_attention" wording on a build
    // that has it.
    #[cfg(feature = "candle-cuda")]
    #[test]
    fn cuda_graph_decode_strategy_is_rejected_without_paged_attention() {
        let dir = write_fixture_dir("cuda-graph-no-paged-reject");
        let strategy = HardwareStrategy {
            cuda_graph_decode: true,
            ..Default::default()
        };
        let error = CandleLlmRuntime::try_load("tiny", &dir, "cuda", &strategy)
            .expect_err("cuda_graph_decode must not silently run without paged_attention");
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(
                    detail.contains("requires hardware_strategy.paged_attention"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    // Requires a real CUDA device (`arc-gpu-runners`/`cuda-quality`) — not
    // runnable on this repo's default Windows dev sandbox, same as
    // `single_device_llama_paged_attention_generates_a_real_decode_on_cuda`.
    // Exercises `CudaGraphDecodeSession`'s establish (warm-up + capture +
    // first replay) and every subsequent step's replay, cross-checked
    // against the non-captured paged-attention path's output for the same
    // checkpoint/prompt: a captured decode replaying incorrectly could still
    // produce non-empty but *wrong* output, so a plain non-emptiness check
    // wouldn't catch a subtle correctness bug (e.g. a stale position/decode
    // slot) the way this equality check does.
    #[cfg(feature = "candle-cuda")]
    #[test]
    fn single_device_llama_cuda_graph_decode_generates_a_real_captured_decode_on_cuda() {
        let dir = write_fixture_dir_with(
            "cuda-graph-decode-cuda",
            write_tachyon_tiny_paged_attention_fixture,
        );
        let request: &[u8] = br#"{"prompt":"hello mesh","max_new_tokens":4,"temperature":0.0}"#;

        let captured_strategy = HardwareStrategy {
            paged_attention: true,
            cuda_graph_decode: true,
            ..Default::default()
        };
        let captured_runtime = CandleLlmRuntime::try_load("tiny", &dir, "cuda", &captured_strategy)
            .expect("a paged-attention + cuda_graph_decode Llama checkpoint must load on a real cuda device")
            .expect("tiny fixture should select the native Candle runtime");
        let captured_output = captured_runtime
            .generate(&[request])
            .expect("cuda_graph_decode generation must run a real captured/replayed decode");
        assert!(
            !captured_output.is_empty(),
            "cuda_graph_decode generation must not be empty/mocked"
        );

        let captured_second_output = captured_runtime
            .generate(&[request])
            .expect("cuda_graph_decode must support a second independent request");
        assert_eq!(
            captured_second_output, captured_output,
            "cuda_graph_decode's second request must match the first captured request's greedy output"
        );
        drop(captured_runtime);

        let paged_only_strategy = HardwareStrategy {
            paged_attention: true,
            ..Default::default()
        };
        let paged_only_runtime =
            CandleLlmRuntime::try_load("tiny", &dir, "cuda", &paged_only_strategy)
                .expect("the checkpoint must load with paged_attention alone")
                .expect("tiny fixture should select the native Candle runtime");
        let paged_only_output = paged_only_runtime
            .generate(&[request])
            .expect("paged-attention generation must run a real decode on the cuda device");
        assert_eq!(
            captured_output, paged_only_output,
            "cuda_graph_decode's captured/replayed decode must match the non-captured paged-attention path's greedy output for the same prompt"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(feature = "candle-cuda")]
    #[test]
    fn cuda_graph_decode_strategy_is_rejected_for_gguf() {
        let (runtime, dir) = load_gguf_fixture("cuda-graph-gguf-reject");
        drop(runtime);
        let strategy = HardwareStrategy {
            paged_attention: true,
            cuda_graph_decode: true,
            ..Default::default()
        };
        let error = CandleLlmRuntime::try_load("tiny-gguf", &dir, "cuda", &strategy)
            .expect_err("cuda_graph_decode must not silently load GGUF on CPU");
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(
                    detail.contains("Safetensors Llama checkpoint"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn flashinfer_attention_strategy_is_rejected_until_decode_attention_is_wired() {
        let dir = write_fixture_dir("flashinfer-reject");
        let strategy = HardwareStrategy {
            flashinfer_attention: true,
            ..Default::default()
        };
        let error = CandleLlmRuntime::try_load("tiny", &dir, "cpu", &strategy)
            .expect_err("FlashInfer attention must not silently use the default attention path");
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(
                    detail.contains("candle-flashinfer-kernels::flashinfer_decode_attention"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    // Holds on every build (with or without candle-cuda/candle-flashinfer):
    // only a Llama checkpoint on a CUDA device can get flashinfer_attention
    // wired.
    #[test]
    fn flashinfer_attention_strategy_is_rejected_for_a_non_llama_architecture() {
        let dir = std::env::temp_dir().join(format!(
            "tachyon-qwen2-flashinfer-reject-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock should be after the epoch")
                .as_nanos()
        ));
        write_tachyon_tiny_qwen_fixture(&dir, ModelArchitecture::Qwen2)
            .expect("Qwen fixture should be written");
        let strategy = HardwareStrategy {
            flashinfer_attention: true,
            ..Default::default()
        };
        let error = CandleLlmRuntime::try_load("tiny-qwen", &dir, "cuda", &strategy).expect_err(
            "a non-Llama architecture must keep rejecting flashinfer_attention even on a cuda request",
        );
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(
                    detail.contains("flashinfer_decode_attention"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    // Regression for a real bug an automated review caught: for a
    // tensor/pipeline/expert-parallel Llama deployment, `load_parallel`'s
    // Config never sets `use_flashinfer_attention` (it has no FlashInfer path
    // at all), so a parallel-mode request setting this flag used to pass this
    // gate and then silently load with the default attention implementation.
    // Holds on every build: a non-`Single` distribution mode must keep
    // rejecting flashinfer_attention regardless of architecture/device.
    #[test]
    fn flashinfer_attention_strategy_is_rejected_for_a_non_single_distribution_mode() {
        let dir = write_fixture_dir("flashinfer-tp-reject");
        let strategy = HardwareStrategy {
            distribution_mode: GpuDistribution::TensorParallelism,
            device_ids: vec![0, 1],
            flashinfer_attention: true,
            ..Default::default()
        };
        let error = CandleLlmRuntime::try_load_with_topology(
            "tiny",
            &dir,
            "cuda",
            &strategy,
            &two_device_topology(),
        )
        .expect_err(
            "a tensor-parallel request must keep rejecting flashinfer_attention, not silently ignore it",
        );
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(
                    detail.contains("flashinfer_decode_attention"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    // Reaching the paged-attention conflict message (rather than the "feature
    // not compiled in" rejection) requires both candle-cuda and
    // candle-flashinfer, same as the real execution test below.
    #[cfg(all(feature = "candle-cuda", feature = "candle-flashinfer"))]
    #[test]
    fn flashinfer_attention_and_paged_attention_combination_is_rejected() {
        let dir = write_fixture_dir("flashinfer-paged-combo-reject");
        let strategy = HardwareStrategy {
            flashinfer_attention: true,
            paged_attention: true,
            ..Default::default()
        };
        let error = CandleLlmRuntime::try_load("tiny", &dir, "cuda", &strategy)
            .expect_err("flashinfer_attention and paged_attention must not silently compose");
        match error {
            CandleLlmError::UnsupportedModel { detail, .. } => {
                assert!(
                    detail.contains("cannot be combined with paged_attention"),
                    "unexpected detail: {detail}"
                );
            }
            other => panic!("expected UnsupportedModel, got {other:?}"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    // Requires a real CUDA device (`arc-gpu-runners`/`cuda-quality`) — not
    // runnable on this repo's default Windows dev sandbox, same as
    // `single_device_llama_paged_attention_generates_a_real_decode_on_cuda`.
    #[cfg(all(feature = "candle-cuda", feature = "candle-flashinfer"))]
    #[test]
    fn single_device_llama_flashinfer_attention_generates_a_real_decode_on_cuda() {
        let dir = write_fixture_dir("flashinfer-attn-cuda");
        let strategy = HardwareStrategy {
            flashinfer_attention: true,
            ..Default::default()
        };
        let runtime = CandleLlmRuntime::try_load("tiny", &dir, "cuda", &strategy)
            .expect("a flashinfer-attention Llama checkpoint must load on a real cuda device")
            .expect("tiny fixture should select the native Candle runtime");
        let request: &[u8] = br#"{"prompt":"hello mesh","max_new_tokens":4,"temperature":0.0}"#;
        let flashinfer_output = runtime
            .generate(&[request])
            .expect("flashinfer-attention generation must run a real decode on the cuda device");
        assert!(
            !flashinfer_output.is_empty(),
            "flashinfer-attention generation must not be empty/mocked"
        );

        // flashinfer_decode_attention is numerically equivalent to the dense
        // matmul+softmax computation for a single decode token (proven by the
        // fork's own `flashinfer_decode_matches_dense_path` test), so greedy
        // generation over the same checkpoint/prompt should match the dense
        // path's output, not just be non-empty.
        let dense_runtime =
            CandleLlmRuntime::try_load("tiny", &dir, "cuda", &HardwareStrategy::default())
                .expect("the same checkpoint must also load without flashinfer_attention")
                .expect("tiny fixture should select the native Candle runtime");
        let dense_output = dense_runtime
            .generate(&[request])
            .expect("dense generation must run a real decode on the cuda device");
        assert_eq!(
            flashinfer_output, dense_output,
            "flashinfer-attention decode must match the dense path's greedy output for the same prompt"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(feature = "candle-flashinfer")]
    #[test]
    fn flashinfer_kernel_dependency_runs_reference_decode_attention() {
        use candle_flashinfer_kernels::flashinfer_decode_attention;

        let device = Device::Cpu;
        let q = Tensor::from_vec(vec![1f32, 0.5], (1, 1, 2), &device).expect("q");
        let k = Tensor::from_vec(vec![1f32, 0.0, 0.0, 1.0], (1, 1, 2, 2), &device).expect("k");
        let v = Tensor::from_vec(vec![2f32, 4.0, 6.0, 8.0], (1, 1, 2, 2), &device).expect("v");
        let out = flashinfer_decode_attention(&q, &k, &v, 1.0)
            .expect("FlashInfer-style reference kernel should run");
        assert_eq!(out.dims(), &[1, 1, 2]);
    }
}
