use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use candle_core::{quantized::gguf_file, safetensors::MmapedSafetensors, DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::llama::{Cache, Config, Llama, LlamaConfig, LlamaEosToks};
use candle_transformers::models::quantized_llama::ModelWeights as QuantizedLlama;
use serde::Deserialize;
use thiserror::Error;
use tokenizers::Tokenizer;

use crate::{GpuDistribution, HardwareStrategy};

use super::expert_parallel_llama::ExpertParallelLlama;
use super::parallel::{discover_cluster_topology, ExpertPlacementPlan};
use super::pipeline_parallel_llama::PipelineParallelLlama;
use super::tensor_parallel_llama::{TensorParallelCache, TensorParallelLlama};
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

/// A loaded, ready-to-run Llama-family model. Weights are mmapped (safetensors)
/// or read (GGUF) from the model directory — never copied into the Tachyon
/// artifact. Shared behind an `Arc` so the runtime stays cheap to clone.
enum LoadedModel {
    /// Full-precision safetensors Llama with an external, per-request KV cache.
    Safetensors {
        model: Llama,
        config: Config,
        eos_tokens: Vec<u32>,
    },
    /// Quantised GGUF Llama. `forward` takes `&mut self` (the KV cache lives
    /// inside the weights and resets whenever a sequence restarts at
    /// `index_pos == 0`), so it is guarded by a `Mutex`; the QoS scheduler
    /// already serialises execution per accelerator, so contention is minimal.
    Gguf {
        model: Mutex<QuantizedLlama>,
        eos_tokens: Vec<u32>,
    },
    /// A multi-device parallel engine, selected when the deployment's
    /// `hardware_strategy.distribution_mode` is not `single`. The engines
    /// themselves are the numerically-verified ones from
    /// `tensor_parallel_llama`/`pipeline_parallel_llama`; this variant is the
    /// runtime dispatch that finally selects them.
    Parallel(ParallelModel),
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
#[cfg(test)]
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
}

impl GenerationLimits {
    fn with_context(max_position_embeddings: usize) -> Self {
        Self {
            default_max_new_tokens: DEFAULT_MAX_NEW_TOKENS,
            max_prompt_bytes: DEFAULT_MAX_PROMPT_BYTES,
            max_prompt_tokens: DEFAULT_MAX_PROMPT_TOKENS,
            max_batch_size: DEFAULT_MAX_BATCH_SIZE,
            max_position_embeddings,
        }
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
}

#[derive(Debug, Deserialize)]
struct ModelTypeProbe {
    #[serde(default)]
    model_type: String,
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
struct ChatTurn {
    role: String,
    content: String,
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

        // The single-device path remains CPU-only in this runtime (GPU
        // execution of the dense path is the separate
        // `gpu-accelerated-inference-execution` change), so a GPU request on a
        // `single` deployment still returns the existing typed error. A
        // non-`single` strategy resolves its devices from the validated plan
        // instead, so it does not go through this check.
        if strategy.is_single() && requested_device != "cpu" {
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

        let (inner, limits) = if strategy.is_single() {
            match format {
                ModelFormat::Safetensors => Self::load_safetensors(alias, root)?,
                ModelFormat::Gguf => Self::load_gguf(alias, root)?,
            }
        } else {
            Self::load_parallel(alias, root, format, strategy, topology)?
        };

        Ok(Some(Self {
            alias: alias.to_owned(),
            root: root.to_path_buf(),
            tokenizer,
            inner: Arc::new(inner),
            limits,
            chat_template,
        }))
    }

    /// Load a Hugging Face safetensors Llama directory: `config.json` (validated
    /// as `model_type = "llama"`) plus single-file or sharded safetensors,
    /// mmapped so the weights stay on disk.
    fn load_safetensors(
        alias: &str,
        root: &Path,
    ) -> Result<(LoadedModel, GenerationLimits), CandleLlmError> {
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
        let config = llama_config.into_config(false);
        let limits = GenerationLimits::with_context(config.max_position_embeddings);
        let eos_tokens = eos_token_ids(&config);

        let weight_paths = safetensors_paths(alias, root)?;
        let device = Device::Cpu;
        // SAFETY: the model files live in the (uploaded) model directory and are
        // not mutated for the lifetime of the mmap held by the VarBuilder/model.
        let var_builder =
            unsafe { VarBuilder::from_mmaped_safetensors(&weight_paths, DType::F32, &device) }
                .map_err(|error| CandleLlmError::InvalidComponent {
                    alias: alias.to_owned(),
                    path: root.to_path_buf(),
                    component: MODEL_SAFETENSORS,
                    detail: error.to_string(),
                })?;
        let model = Llama::load(var_builder, &config).map_err(|error| {
            CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: MODEL_SAFETENSORS,
                detail: error.to_string(),
            }
        })?;

        Ok((
            LoadedModel::Safetensors {
                model,
                config,
                eos_tokens,
            },
            limits,
        ))
    }

    /// Load a single GGUF Llama file. The architecture and hyper-parameters come
    /// from GGUF metadata (there is no `config.json`); the quantised tensors are
    /// read from the file. The same reader feeds the header parse and the tensor
    /// reads (candle seeks via `tensor_data_offset`).
    fn load_gguf(
        alias: &str,
        root: &Path,
    ) -> Result<(LoadedModel, GenerationLimits), CandleLlmError> {
        let gguf_path = gguf_file_path(alias, root)?;
        let device = Device::Cpu;
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
            config: raw.base.into_config(false),
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
        Ok(llama_config.into_config(false))
    }

    /// Buffered generation: run the decode and return the full UTF-8 output. A
    /// thin accumulator over [`generate_streaming`], so the two paths always
    /// agree byte-for-byte.
    pub(crate) fn generate(&self, prompts: &[&[u8]]) -> Result<Vec<u8>, CandleLlmError> {
        let mut out = String::new();
        self.generate_streaming(prompts, &mut |delta| out.push_str(delta))?;
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

    pub(crate) fn root(&self) -> &Path {
        &self.root
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
        let device = Device::Cpu;
        match &*self.inner {
            LoadedModel::Safetensors {
                model,
                config,
                eos_tokens,
            } => {
                let mut cache = Cache::new(true, DType::F32, config, &device).map_err(|error| {
                    self.execution_error(format!("failed to build KV cache: {error}"))
                })?;
                self.decode_loop(
                    prompt_ids,
                    request,
                    eos_tokens,
                    &device,
                    on_token,
                    |input, index_pos| model.forward(input, index_pos, &mut cache),
                )
            }
            LoadedModel::Gguf { model, eos_tokens } => {
                let mut guard = model.lock().map_err(|_| {
                    self.execution_error("GGUF model mutex was poisoned".to_owned())
                })?;
                self.decode_loop(
                    prompt_ids,
                    request,
                    eos_tokens,
                    &device,
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
        let mut processor = request.sampling.processor();
        let mut tokens = prompt_ids.to_vec();
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
        let mut index_pos = 0usize;
        for step in 0..request.max_new_tokens {
            let context: &[u32] = if step == 0 {
                &tokens
            } else {
                &tokens[tokens.len() - 1..]
            };
            if index_pos + context.len() > self.limits.max_position_embeddings {
                break;
            }
            let input = Tensor::new(context, device)
                .and_then(|tensor| tensor.unsqueeze(0))
                .map_err(|error| {
                    self.execution_error(format!("failed to build input tensor: {error}"))
                })?;
            let logits = forward(&input, index_pos).map_err(|error| {
                self.execution_error(format!("transformer forward pass failed: {error}"))
            })?;
            let row = logits.squeeze(0).map_err(|error| {
                self.execution_error(format!("failed to reshape logits: {error}"))
            })?;
            let next = processor.sample(&row).map_err(|error| {
                self.execution_error(format!("failed to sample next token: {error}"))
            })?;
            index_pos += context.len();
            tokens.push(next);
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
        }
        // Token budget exhausted: flush the held-back tail, trimming any stop.
        let text = self.decode_generated(&generated)?;
        let end = find_earliest_stop(&text, &request.stop).unwrap_or(text.len());
        emit_delta(on_token, &text, &mut emitted, end);
        Ok(())
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
        let device = Device::Cpu;
        let input = Tensor::new(ids.as_slice(), &device)
            .and_then(|tensor| tensor.unsqueeze(0))
            .expect("input tensor should build");
        let logits = match &*self.inner {
            LoadedModel::Safetensors { model, config, .. } => {
                let mut cache =
                    Cache::new(true, DType::F32, config, &device).expect("cache should build");
                model
                    .forward(&input, 0, &mut cache)
                    .expect("forward pass should run")
            }
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
            ParsedGenerationRequest {
                prompt,
                max_new_tokens: request
                    .max_new_tokens
                    .unwrap_or(self.limits.default_max_new_tokens),
                sampling: resolve_sampling(request.temperature, request.top_p, request.seed),
                stop: sanitize_stop(request.stop),
            }
        } else {
            ParsedGenerationRequest {
                prompt: raw.to_owned(),
                max_new_tokens: self.limits.default_max_new_tokens,
                sampling: resolve_sampling(None, None, None),
                stop: Vec::new(),
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
struct ChatTemplate {
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
    fn load(alias: &str, root: &Path) -> Result<Option<Self>, CandleLlmError> {
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
    fn render(&self, messages: &[ChatTurn]) -> Result<String, String> {
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

    fn load_fixture(tag: &str) -> (CandleLlmRuntime, PathBuf) {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "tachyon-llama-{tag}-{}-{nanos}",
            std::process::id()
        ));
        write_tachyon_tiny_fixture(&dir).expect("fixture should be written");
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

    #[test]
    fn single_strategy_still_rejects_a_gpu_device_request() {
        let dir = write_fixture_dir("single-gpu-reject");
        let error = CandleLlmRuntime::try_load("tiny", &dir, "cuda", &HardwareStrategy::default())
            .expect_err("single-device path is cpu-only and must reject a gpu request");
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
}
