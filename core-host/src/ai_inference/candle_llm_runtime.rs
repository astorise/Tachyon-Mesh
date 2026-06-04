use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::llama::{Cache, Config, Llama, LlamaConfig, LlamaEosToks};
use serde::Deserialize;
use thiserror::Error;
use tokenizers::Tokenizer;

/// HF `model_type` of the only architecture family currently executed. Real,
/// uploaded Llama-family checkpoints (Llama 2/3, TinyLlama, Vicuna, …) carry
/// this value in their `config.json`.
pub(crate) const LLAMA_MODEL_TYPE: &str = "llama";

const CONFIG_JSON: &str = "config.json";
const TOKENIZER_JSON: &str = "tokenizer.json";
const MODEL_SAFETENSORS: &str = "model.safetensors";
const SAFETENSORS_INDEX_JSON: &str = "model.safetensors.index.json";
const DEFAULT_MAX_NEW_TOKENS: usize = 64;
/// Hard upper bound on `max_new_tokens` for any single request, regardless of
/// what the caller asks for. Protects the host from unbounded decode loops.
pub(crate) const HOST_MAX_NEW_TOKENS: usize = 256;
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

/// A loaded, ready-to-run Llama-family model. The weights are mmapped from the
/// model directory (never copied into the Tachyon artifact); this struct is
/// shared behind an `Arc` so the runtime stays cheap to clone.
struct LoadedModel {
    model: Llama,
    config: Config,
    eos_tokens: Vec<u32>,
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
    fn for_config(config: &Config) -> Self {
        Self {
            default_max_new_tokens: DEFAULT_MAX_NEW_TOKENS,
            max_prompt_bytes: DEFAULT_MAX_PROMPT_BYTES,
            max_prompt_tokens: DEFAULT_MAX_PROMPT_TOKENS,
            max_batch_size: DEFAULT_MAX_BATCH_SIZE,
            max_position_embeddings: config.max_position_embeddings,
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
}

#[derive(Debug, Deserialize)]
struct ModelTypeProbe {
    #[serde(default)]
    model_type: String,
}

#[derive(Debug, Deserialize)]
struct SafetensorsIndex {
    weight_map: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct GenerationRequest {
    prompt: String,
    max_new_tokens: Option<usize>,
    temperature: Option<f32>,
    seed: Option<u64>,
}

struct ParsedGenerationRequest {
    prompt: String,
    max_new_tokens: usize,
    _temperature: Option<f32>,
    _seed: Option<u64>,
}

impl CandleLlmRuntime {
    pub(crate) fn try_load(
        alias: &str,
        path: impl AsRef<Path>,
        requested_device: &str,
    ) -> Result<Option<Self>, CandleLlmError> {
        let root = path.as_ref();
        if !root.is_dir() {
            return Ok(None);
        }

        let has_config = root.join(CONFIG_JSON).exists();
        let has_tokenizer = root.join(TOKENIZER_JSON).exists();
        let has_weights =
            root.join(MODEL_SAFETENSORS).exists() || root.join(SAFETENSORS_INDEX_JSON).exists();
        if !(has_config || has_tokenizer || has_weights) {
            return Ok(None);
        }

        if !has_config && has_tokenizer {
            return Err(CandleLlmError::MissingFile {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                file: CONFIG_JSON,
            });
        }
        if !has_config {
            return Ok(None);
        }

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

        if requested_device != "cpu" {
            return Err(CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail: format!(
                    "the Candle LLM runtime supports `cpu` execution only, got `{requested_device}`"
                ),
            });
        }

        if !has_tokenizer {
            return Err(CandleLlmError::MissingFile {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                file: TOKENIZER_JSON,
            });
        }
        if !has_weights {
            return Err(CandleLlmError::MissingFile {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                file: MODEL_SAFETENSORS,
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

        let llama_config: LlamaConfig = serde_json::from_slice(&raw_config).map_err(|error| {
            CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: CONFIG_JSON,
                detail: error.to_string(),
            }
        })?;
        let config = llama_config.into_config(false);
        let limits = GenerationLimits::for_config(&config);
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

        Ok(Some(Self {
            alias: alias.to_owned(),
            root: root.to_path_buf(),
            tokenizer,
            inner: Arc::new(LoadedModel {
                model,
                config,
                eos_tokens,
            }),
            limits,
        }))
    }

    pub(crate) fn generate(&self, prompts: &[&[u8]]) -> Result<Vec<u8>, CandleLlmError> {
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

        let generated = self.decode_greedy(&prompt_ids, request.max_new_tokens)?;
        let text =
            self.tokenizer
                .decode(&generated, true)
                .map_err(|error| CandleLlmError::Execution {
                    alias: self.alias.clone(),
                    detail: format!("failed to decode generated tokens: {error}"),
                })?;
        Ok(text.into_bytes())
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// Greedy autoregressive decode through the real Llama forward, reusing a
    /// fresh KV cache per request: the prompt is processed once, then each new
    /// token is fed back in with the running `index_pos`.
    fn decode_greedy(
        &self,
        prompt_ids: &[u32],
        max_new_tokens: usize,
    ) -> Result<Vec<u32>, CandleLlmError> {
        let device = Device::Cpu;
        let mut cache = Cache::new(true, DType::F32, &self.inner.config, &device)
            .map_err(|error| self.execution_error(format!("failed to build KV cache: {error}")))?;
        let mut tokens = prompt_ids.to_vec();
        let mut generated = Vec::with_capacity(max_new_tokens);
        let mut index_pos = 0usize;
        for step in 0..max_new_tokens {
            let context: &[u32] = if step == 0 {
                &tokens
            } else {
                &tokens[tokens.len() - 1..]
            };
            if index_pos + context.len() > self.limits.max_position_embeddings {
                break;
            }
            let next = self.forward_argmax(context, index_pos, &mut cache)?;
            index_pos += context.len();
            tokens.push(next);
            generated.push(next);
            if self.inner.eos_tokens.contains(&next) {
                break;
            }
        }
        Ok(generated)
    }

    /// Run one forward step over `context` and greedily pick the next token id.
    fn forward_argmax(
        &self,
        context: &[u32],
        index_pos: usize,
        cache: &mut Cache,
    ) -> Result<u32, CandleLlmError> {
        let input = Tensor::new(context, &Device::Cpu)
            .and_then(|tensor| tensor.unsqueeze(0))
            .map_err(|error| {
                self.execution_error(format!("failed to build input tensor: {error}"))
            })?;
        let logits = self
            .inner
            .model
            .forward(&input, index_pos, cache)
            .map_err(|error| {
                self.execution_error(format!("transformer forward pass failed: {error}"))
            })?;
        logits
            .squeeze(0)
            .and_then(|row| row.argmax(0))
            .and_then(|id| id.to_vec0::<u32>())
            .map_err(|error| {
                self.execution_error(format!("failed to sample greedy token: {error}"))
            })
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

    /// Test-only: the vocabulary logits the model produces for the final
    /// position of `prompt`. Used to prove generation is prompt-dependent.
    #[cfg(test)]
    pub(crate) fn debug_last_logits(&self, prompt: &str) -> Vec<f32> {
        let ids = self.encode_ids(prompt).expect("prompt should tokenize");
        let device = Device::Cpu;
        let mut cache =
            Cache::new(true, DType::F32, &self.inner.config, &device).expect("cache should build");
        let input = Tensor::new(ids.as_slice(), &device)
            .and_then(|tensor| tensor.unsqueeze(0))
            .expect("input tensor should build");
        let logits = self
            .inner
            .model
            .forward(&input, 0, &mut cache)
            .expect("forward pass should run");
        logits
            .squeeze(0)
            .and_then(|row| row.to_vec1::<f32>())
            .expect("logits should be an f32 vector")
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
            ParsedGenerationRequest {
                prompt: request.prompt,
                max_new_tokens: request
                    .max_new_tokens
                    .unwrap_or(self.limits.default_max_new_tokens),
                _temperature: request.temperature,
                _seed: request.seed,
            }
        } else {
            ParsedGenerationRequest {
                prompt: raw.to_owned(),
                max_new_tokens: self.limits.default_max_new_tokens,
                _temperature: None,
                _seed: None,
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

/// Collect the model's end-of-sequence token id(s), if any, into a flat list.
fn eos_token_ids(config: &Config) -> Vec<u32> {
    match &config.eos_token_id {
        Some(LlamaEosToks::Single(id)) => vec![*id],
        Some(LlamaEosToks::Multiple(ids)) => ids.clone(),
        None => Vec::new(),
    }
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
    fs::write(
        root.join(TOKENIZER_JSON),
        r#"{
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
}"#,
    )?;
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
        let runtime = CandleLlmRuntime::try_load("tiny", &dir, "cpu")
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
        fs::write(dir.join(TOKENIZER_JSON), b"{}").expect("tokenizer marker");
        fs::write(dir.join(MODEL_SAFETENSORS), b"marker").expect("weights marker");

        let error = CandleLlmRuntime::try_load("tiny", &dir, "cpu")
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

        let error = CandleLlmRuntime::try_load("tiny", &dir, "cpu")
            .expect_err("a model missing lm_head.weight must fail to load");
        assert!(matches!(error, CandleLlmError::InvalidComponent { .. }));
        let _ = fs::remove_dir_all(dir);
    }
}
