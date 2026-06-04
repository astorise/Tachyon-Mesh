use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use candle_core::{safetensors, DType, Device, Tensor};
use candle_nn::{Embedding, LayerNorm, Linear, Module, VarBuilder};
use serde::Deserialize;
use thiserror::Error;
use tokenizers::Tokenizer;

pub(crate) const TACHYON_TINY_MODEL_TYPE: &str = "tachyon_tiny_causal_lm";
pub(crate) const TACHYON_TINY_ARCHITECTURE: &str = "TachyonTinyCausalLM";

const CONFIG_JSON: &str = "config.json";
const TOKENIZER_JSON: &str = "tokenizer.json";
const MODEL_SAFETENSORS: &str = "model.safetensors";
const DEFAULT_MAX_NEW_TOKENS: usize = 1;
const HOST_MAX_NEW_TOKENS: usize = 64;
const DEFAULT_MAX_PROMPT_BYTES: usize = 4096;
const DEFAULT_MAX_PROMPT_TOKENS: usize = 1024;
const DEFAULT_MAX_BATCH_SIZE: usize = 32;
const LAYER_NORM_EPS: f64 = 1e-5;

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

#[derive(Clone)]
pub(crate) struct CandleLlmRuntime {
    alias: String,
    root: PathBuf,
    tokenizer: Tokenizer,
    config: CandleLlmConfig,
    model: Arc<TachyonTinyModel>,
}

#[derive(Clone, Debug, Deserialize)]
struct CandleLlmConfig {
    model_type: String,
    #[serde(default)]
    architectures: Vec<String>,
    vocab_size: usize,
    #[serde(default = "default_hidden_size")]
    hidden_size: usize,
    #[serde(default = "default_num_hidden_layers")]
    num_hidden_layers: usize,
    #[serde(default = "default_num_attention_heads")]
    num_attention_heads: usize,
    #[serde(default = "default_intermediate_size")]
    intermediate_size: usize,
    #[serde(default = "default_max_position_embeddings")]
    max_position_embeddings: usize,
    #[serde(default = "default_max_new_tokens")]
    default_max_new_tokens: usize,
    #[serde(default = "default_max_prompt_bytes")]
    max_prompt_bytes: usize,
    #[serde(default = "default_max_prompt_tokens")]
    max_prompt_tokens: usize,
    #[serde(default = "default_max_batch_size")]
    max_batch_size: usize,
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
        let has_weights = root.join(MODEL_SAFETENSORS).exists();
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

        let config = load_config(alias, root)?;
        if config.model_type != TACHYON_TINY_MODEL_TYPE
            || !config
                .architectures
                .iter()
                .any(|name| name == TACHYON_TINY_ARCHITECTURE)
        {
            return Err(CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail: format!(
                    "expected model_type `{}` with architecture `{}`",
                    TACHYON_TINY_MODEL_TYPE, TACHYON_TINY_ARCHITECTURE
                ),
            });
        }

        if requested_device != "cpu" {
            return Err(CandleLlmError::UnsupportedModel {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                detail: format!(
                    "the first Candle LLM runtime supports `cpu` execution only, got `{requested_device}`"
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

        let tensors =
            safetensors::load(root.join(MODEL_SAFETENSORS), &Device::Cpu).map_err(|error| {
                CandleLlmError::InvalidComponent {
                    alias: alias.to_owned(),
                    path: root.to_path_buf(),
                    component: MODEL_SAFETENSORS,
                    detail: error.to_string(),
                }
            })?;
        let model = build_model(alias, root, &config, tensors)?;

        Ok(Some(Self {
            alias: alias.to_owned(),
            root: root.to_path_buf(),
            tokenizer,
            config,
            model: Arc::new(model),
        }))
    }

    pub(crate) fn generate(&self, prompts: &[&[u8]]) -> Result<Vec<u8>, CandleLlmError> {
        if prompts.is_empty() {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: "at least one U8 prompt tensor is required".to_owned(),
            });
        }
        if prompts.len() > self.config.max_batch_size {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!(
                    "batch size {} exceeds max batch size {}",
                    prompts.len(),
                    self.config.max_batch_size
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

        // Real autoregressive decode: the prompt tokens flow through the
        // transformer, and each step appends the greedily-argmaxed next token to
        // the running context before recomputing — so the output is a genuine
        // function of the prompt, not a static lookup.
        let prompt_ids = self.encode_ids(&request.prompt)?;
        if prompt_ids.is_empty() {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: "prompt produced no tokens to condition on".to_owned(),
            });
        }
        let mut context = prompt_ids;
        let mut generated = Vec::with_capacity(request.max_new_tokens);
        for _ in 0..request.max_new_tokens {
            if context.len() >= self.config.max_position_embeddings {
                break;
            }
            let logits =
                self.model
                    .forward(&context)
                    .map_err(|error| CandleLlmError::Execution {
                        alias: self.alias.clone(),
                        detail: format!("transformer forward pass failed: {error}"),
                    })?;
            let next = logits
                .argmax(0)
                .and_then(|id| id.to_vec0::<u32>())
                .map_err(|error| CandleLlmError::Execution {
                    alias: self.alias.clone(),
                    detail: format!("failed to sample greedy token with Candle: {error}"),
                })?;
            generated.push(next);
            context.push(next);
        }

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

    /// Test-only: the vocabulary logits the transformer produces for the final
    /// position of `prompt`. Used to prove that generation is a genuine function
    /// of the prompt (different prompts -> different logits), not a static lookup.
    #[cfg(test)]
    pub(crate) fn debug_last_logits(&self, prompt: &str) -> Vec<f32> {
        let ids = self.encode_ids(prompt).expect("prompt should tokenize");
        let logits = self.model.forward(&ids).expect("forward pass should run");
        logits
            .to_vec1::<f32>()
            .expect("last-position logits should be an f32 vector")
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

    fn parse_request(&self, data: &[u8]) -> Result<ParsedGenerationRequest, CandleLlmError> {
        if data.len() > self.config.max_prompt_bytes {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!(
                    "prompt bytes {} exceed limit {}",
                    data.len(),
                    self.config.max_prompt_bytes
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
                    .unwrap_or(self.config.default_max_new_tokens),
                _temperature: request.temperature,
                _seed: request.seed,
            }
        } else {
            ParsedGenerationRequest {
                prompt: raw.to_owned(),
                max_new_tokens: self.config.default_max_new_tokens,
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
        if encoded.len() > self.config.max_prompt_tokens
            || encoded.len() > self.config.max_position_embeddings
        {
            return Err(CandleLlmError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!(
                    "prompt tokens {} exceed token limit {} and context limit {}",
                    encoded.len(),
                    self.config.max_prompt_tokens,
                    self.config.max_position_embeddings
                ),
            });
        }

        Ok(request)
    }
}

/// A minimal but genuine decoder-only transformer (`TachyonTinyCausalLM`):
/// token + learned positional embeddings, `num_hidden_layers` pre-norm blocks of
/// causal multi-head self-attention and a GELU MLP, a final layer norm, and an
/// untied `lm_head`. Built from `model.safetensors` via `VarBuilder` and run on
/// CPU in F32.
struct TachyonTinyModel {
    wte: Embedding,
    wpe: Embedding,
    blocks: Vec<TransformerBlock>,
    ln_f: LayerNorm,
    lm_head: Linear,
    num_heads: usize,
    head_dim: usize,
}

impl TachyonTinyModel {
    fn load(vb: &VarBuilder, config: &CandleLlmConfig) -> candle_core::Result<Self> {
        let hidden = config.hidden_size;
        let wte = candle_nn::embedding(config.vocab_size, hidden, vb.pp("wte"))?;
        let wpe = candle_nn::embedding(config.max_position_embeddings, hidden, vb.pp("wpe"))?;
        let layers = vb.pp("layers");
        let mut blocks = Vec::with_capacity(config.num_hidden_layers);
        for index in 0..config.num_hidden_layers {
            blocks.push(TransformerBlock::load(&layers.pp(index), config)?);
        }
        let ln_f = candle_nn::layer_norm(hidden, LAYER_NORM_EPS, vb.pp("ln_f"))?;
        let lm_head = candle_nn::linear_no_bias(hidden, config.vocab_size, vb.pp("lm_head"))?;
        Ok(Self {
            wte,
            wpe,
            blocks,
            ln_f,
            lm_head,
            num_heads: config.num_attention_heads,
            head_dim: hidden / config.num_attention_heads,
        })
    }

    /// Forward the full token context and return the logits over the vocabulary
    /// for the final position only (`[vocab_size]`), which greedy decode argmaxes.
    fn forward(&self, ids: &[u32]) -> candle_core::Result<Tensor> {
        let device = Device::Cpu;
        let seq = ids.len();
        let token_ids = Tensor::from_vec(ids.to_vec(), (seq,), &device)?;
        let position_ids =
            Tensor::from_vec((0..seq as u32).collect::<Vec<u32>>(), (seq,), &device)?;
        // hidden states: token embedding + learned positional embedding.
        let mut hidden = self
            .wte
            .forward(&token_ids)?
            .broadcast_add(&self.wpe.forward(&position_ids)?)?;
        let mask = causal_mask(seq, &device)?;
        for block in &self.blocks {
            hidden = block.forward(&hidden, &mask, self.num_heads, self.head_dim)?;
        }
        let hidden = self.ln_f.forward(&hidden)?;
        let logits = self.lm_head.forward(&hidden)?; // [seq, vocab]
        logits.narrow(0, seq - 1, 1)?.squeeze(0)
    }
}

/// One pre-norm transformer decoder block.
struct TransformerBlock {
    ln1: LayerNorm,
    q: Linear,
    k: Linear,
    v: Linear,
    o: Linear,
    ln2: LayerNorm,
    fc: Linear,
    proj: Linear,
}

impl TransformerBlock {
    fn load(vb: &VarBuilder, config: &CandleLlmConfig) -> candle_core::Result<Self> {
        let hidden = config.hidden_size;
        let attn = vb.pp("attn");
        let mlp = vb.pp("mlp");
        Ok(Self {
            ln1: candle_nn::layer_norm(hidden, LAYER_NORM_EPS, vb.pp("ln1"))?,
            q: candle_nn::linear(hidden, hidden, attn.pp("q"))?,
            k: candle_nn::linear(hidden, hidden, attn.pp("k"))?,
            v: candle_nn::linear(hidden, hidden, attn.pp("v"))?,
            o: candle_nn::linear(hidden, hidden, attn.pp("o"))?,
            ln2: candle_nn::layer_norm(hidden, LAYER_NORM_EPS, vb.pp("ln2"))?,
            fc: candle_nn::linear(hidden, config.intermediate_size, mlp.pp("fc"))?,
            proj: candle_nn::linear(config.intermediate_size, hidden, mlp.pp("proj"))?,
        })
    }

    fn forward(
        &self,
        hidden: &Tensor,
        mask: &Tensor,
        num_heads: usize,
        head_dim: usize,
    ) -> candle_core::Result<Tensor> {
        let attention = self.attention(&self.ln1.forward(hidden)?, mask, num_heads, head_dim)?;
        let hidden = hidden.broadcast_add(&attention)?;
        let mlp = self.mlp(&self.ln2.forward(&hidden)?)?;
        hidden.broadcast_add(&mlp)
    }

    fn attention(
        &self,
        x: &Tensor,
        mask: &Tensor,
        num_heads: usize,
        head_dim: usize,
    ) -> candle_core::Result<Tensor> {
        let seq = x.dim(0)?;
        // Project and split into heads: [seq, hidden] -> [num_heads, seq, head_dim].
        let q = split_heads(&self.q.forward(x)?, seq, num_heads, head_dim)?;
        let k = split_heads(&self.k.forward(x)?, seq, num_heads, head_dim)?;
        let v = split_heads(&self.v.forward(x)?, seq, num_heads, head_dim)?;
        // Scaled dot-product attention with a causal mask, per head.
        let scores = q
            .matmul(&k.transpose(1, 2)?)?
            .affine(1.0 / (head_dim as f64).sqrt(), 0.0)?
            .broadcast_add(mask)?;
        let weights = candle_nn::ops::softmax_last_dim(&scores)?;
        let context = weights.matmul(&v)?; // [num_heads, seq, head_dim]
                                           // Merge heads back: [num_heads, seq, head_dim] -> [seq, hidden].
        let merged = context
            .transpose(0, 1)?
            .contiguous()?
            .reshape((seq, num_heads * head_dim))?;
        self.o.forward(&merged)
    }

    fn mlp(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let hidden = self.fc.forward(x)?.gelu()?;
        self.proj.forward(&hidden)
    }
}

fn split_heads(
    x: &Tensor,
    seq: usize,
    num_heads: usize,
    head_dim: usize,
) -> candle_core::Result<Tensor> {
    x.reshape((seq, num_heads, head_dim))?
        .transpose(0, 1)?
        .contiguous()
}

/// Additive causal mask `[seq, seq]`: `0` where a query may attend (`j <= i`) and
/// `-inf` for future positions (`j > i`), applied before the softmax.
fn causal_mask(seq: usize, device: &Device) -> candle_core::Result<Tensor> {
    let mut data = vec![0f32; seq * seq];
    for i in 0..seq {
        for value in data.iter_mut().skip(i * seq + i + 1).take(seq - i - 1) {
            *value = f32::NEG_INFINITY;
        }
    }
    Tensor::from_vec(data, (seq, seq), device)
}

fn build_model(
    alias: &str,
    root: &Path,
    config: &CandleLlmConfig,
    tensors: HashMap<String, Tensor>,
) -> Result<TachyonTinyModel, CandleLlmError> {
    let invalid = |detail: String| CandleLlmError::InvalidComponent {
        alias: alias.to_owned(),
        path: root.to_path_buf(),
        component: MODEL_SAFETENSORS,
        detail,
    };
    if config.hidden_size == 0
        || config.num_attention_heads == 0
        || !config.hidden_size.is_multiple_of(config.num_attention_heads)
    {
        return Err(invalid(format!(
            "hidden_size {} must be a non-zero multiple of num_attention_heads {}",
            config.hidden_size, config.num_attention_heads
        )));
    }
    let vb = VarBuilder::from_tensors(tensors, DType::F32, &Device::Cpu);
    TachyonTinyModel::load(&vb, config).map_err(|error| invalid(error.to_string()))
}

fn load_config(alias: &str, root: &Path) -> Result<CandleLlmConfig, CandleLlmError> {
    let path = root.join(CONFIG_JSON);
    let data = fs::read(&path).map_err(|error| CandleLlmError::InvalidComponent {
        alias: alias.to_owned(),
        path: root.to_path_buf(),
        component: CONFIG_JSON,
        detail: error.to_string(),
    })?;
    serde_json::from_slice::<CandleLlmConfig>(&data).map_err(|error| {
        CandleLlmError::InvalidComponent {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            component: CONFIG_JSON,
            detail: error.to_string(),
        }
    })
}

fn default_hidden_size() -> usize {
    64
}

fn default_num_hidden_layers() -> usize {
    2
}

fn default_num_attention_heads() -> usize {
    4
}

fn default_intermediate_size() -> usize {
    128
}

fn default_max_position_embeddings() -> usize {
    DEFAULT_MAX_PROMPT_TOKENS
}

fn default_max_new_tokens() -> usize {
    DEFAULT_MAX_NEW_TOKENS
}

fn default_max_prompt_bytes() -> usize {
    DEFAULT_MAX_PROMPT_BYTES
}

fn default_max_prompt_tokens() -> usize {
    DEFAULT_MAX_PROMPT_TOKENS
}

fn default_max_batch_size() -> usize {
    DEFAULT_MAX_BATCH_SIZE
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
pub(crate) const FIXTURE_MAX_POSITION_EMBEDDINGS: usize = 16;

#[cfg(test)]
pub(crate) fn write_tachyon_tiny_fixture(root: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join(CONFIG_JSON),
        serde_json::json!({
            "model_type": TACHYON_TINY_MODEL_TYPE,
            "architectures": [TACHYON_TINY_ARCHITECTURE],
            "vocab_size": FIXTURE_VOCAB_SIZE,
            "hidden_size": FIXTURE_HIDDEN_SIZE,
            "num_hidden_layers": FIXTURE_NUM_LAYERS,
            "num_attention_heads": FIXTURE_NUM_HEADS,
            "intermediate_size": FIXTURE_INTERMEDIATE_SIZE,
            "max_position_embeddings": FIXTURE_MAX_POSITION_EMBEDDINGS,
            "default_max_new_tokens": 1,
            "max_prompt_bytes": 128,
            "max_prompt_tokens": 16,
            "max_batch_size": 32
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
    safetensors::save(&fixture_weights()?, root.join(MODEL_SAFETENSORS))?;
    Ok(())
}

/// Build a complete, deterministic, non-degenerate weight set for the fixture
/// model so tests exercise a real forward pass (not random per-run).
#[cfg(test)]
fn fixture_weights() -> anyhow::Result<HashMap<String, Tensor>> {
    let hidden = FIXTURE_HIDDEN_SIZE;
    let inter = FIXTURE_INTERMEDIATE_SIZE;
    let mut tensors = HashMap::new();
    let mut seed = 1u64;
    let mut weight =
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
    let ones = |tensors: &mut HashMap<String, Tensor>, name: &str| -> anyhow::Result<()> {
        tensors.insert(
            name.to_owned(),
            Tensor::from_vec(vec![1f32; hidden], (hidden,), &Device::Cpu)?,
        );
        Ok(())
    };
    let zeros =
        |tensors: &mut HashMap<String, Tensor>, name: &str, len: usize| -> anyhow::Result<()> {
            tensors.insert(
                name.to_owned(),
                Tensor::from_vec(vec![0f32; len], (len,), &Device::Cpu)?,
            );
            Ok(())
        };

    weight(&mut tensors, "wte.weight", &[FIXTURE_VOCAB_SIZE, hidden])?;
    weight(
        &mut tensors,
        "wpe.weight",
        &[FIXTURE_MAX_POSITION_EMBEDDINGS, hidden],
    )?;
    for layer in 0..FIXTURE_NUM_LAYERS {
        // LayerNorm: unit scale, zero shift (degenerate-safe).
        ones(&mut tensors, &format!("layers.{layer}.ln1.weight"))?;
        zeros(&mut tensors, &format!("layers.{layer}.ln1.bias"), hidden)?;
        ones(&mut tensors, &format!("layers.{layer}.ln2.weight"))?;
        zeros(&mut tensors, &format!("layers.{layer}.ln2.bias"), hidden)?;
        for proj in ["q", "k", "v", "o"] {
            weight(
                &mut tensors,
                &format!("layers.{layer}.attn.{proj}.weight"),
                &[hidden, hidden],
            )?;
            zeros(
                &mut tensors,
                &format!("layers.{layer}.attn.{proj}.bias"),
                hidden,
            )?;
        }
        weight(
            &mut tensors,
            &format!("layers.{layer}.mlp.fc.weight"),
            &[inter, hidden],
        )?;
        zeros(&mut tensors, &format!("layers.{layer}.mlp.fc.bias"), inter)?;
        weight(
            &mut tensors,
            &format!("layers.{layer}.mlp.proj.weight"),
            &[hidden, inter],
        )?;
        zeros(
            &mut tensors,
            &format!("layers.{layer}.mlp.proj.bias"),
            hidden,
        )?;
    }
    ones(&mut tensors, "ln_f.weight")?;
    zeros(&mut tensors, "ln_f.bias", hidden)?;
    weight(
        &mut tensors,
        "lm_head.weight",
        &[FIXTURE_VOCAB_SIZE, hidden],
    )?;
    Ok(tensors)
}

/// Deterministic small weights in roughly `[-0.4, 0.4)`, distinct per element and
/// per tensor (via `seed`), so the forward pass is well-conditioned and varies
/// with the input rather than collapsing.
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
            "tachyon-candle-{tag}-{}-{nanos}",
            std::process::id()
        ));
        write_tachyon_tiny_fixture(&dir).expect("fixture should be written");
        let runtime = CandleLlmRuntime::try_load("tiny", &dir, "cpu")
            .expect("fixture should load without error")
            .expect("fixture is a supported Candle LLM model");
        (runtime, dir)
    }

    #[test]
    fn generate_runs_a_real_forward_and_is_not_a_mock() {
        let (runtime, dir) = load_fixture("real-forward");
        let bytes = runtime
            .generate(&[&b"hello"[..]])
            .expect("generation should run the transformer");
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
    fn missing_weight_tensor_is_a_clear_error_not_a_panic() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock should be after the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "tachyon-candle-missing-weight-{}-{nanos}",
            std::process::id()
        ));
        write_tachyon_tiny_fixture(&dir).expect("fixture should be written");
        // Drop one required tensor, then re-save, so the model build fails cleanly.
        let mut tensors = safetensors::load(dir.join(MODEL_SAFETENSORS), &Device::Cpu)
            .expect("fixture weights should load");
        tensors.remove("lm_head.weight");
        safetensors::save(&tensors, dir.join(MODEL_SAFETENSORS))
            .expect("trimmed weights should save");

        let error = match CandleLlmRuntime::try_load("tiny", &dir, "cpu") {
            Ok(_) => panic!("a model missing lm_head.weight must fail to load"),
            Err(error) => error,
        };
        assert!(matches!(error, CandleLlmError::InvalidComponent { .. }));
        let _ = fs::remove_dir_all(dir);
    }
}
