use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use candle_core::{safetensors, DType, Device, Tensor};
use serde::Deserialize;
use thiserror::Error;
use tokenizers::Tokenizer;

pub(crate) const TACHYON_TINY_MODEL_TYPE: &str = "tachyon_tiny_causal_lm";
pub(crate) const TACHYON_TINY_ARCHITECTURE: &str = "TachyonTinyCausalLM";

const CONFIG_JSON: &str = "config.json";
const TOKENIZER_JSON: &str = "tokenizer.json";
const MODEL_SAFETENSORS: &str = "model.safetensors";
const NEXT_TOKEN_LOGITS: &str = "next_token_logits";
const DEFAULT_MAX_NEW_TOKENS: usize = 1;
const HOST_MAX_NEW_TOKENS: usize = 64;
const DEFAULT_MAX_PROMPT_BYTES: usize = 4096;
const DEFAULT_MAX_PROMPT_TOKENS: usize = 1024;
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

#[derive(Clone)]
pub(crate) struct CandleLlmRuntime {
    alias: String,
    root: PathBuf,
    tokenizer: Tokenizer,
    config: CandleLlmConfig,
    next_token_logits: Tensor,
}

#[derive(Clone, Debug, Deserialize)]
struct CandleLlmConfig {
    model_type: String,
    #[serde(default)]
    architectures: Vec<String>,
    vocab_size: usize,
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
        let next_token_logits = load_logits(alias, root, &config, tensors)?;

        Ok(Some(Self {
            alias: alias.to_owned(),
            root: root.to_path_buf(),
            tokenizer,
            config,
            next_token_logits,
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

        let output_ids = (0..request.max_new_tokens)
            .map(|_| self.next_token_id())
            .collect::<Result<Vec<_>, _>>()?;
        let text = self.tokenizer.decode(&output_ids, true).map_err(|error| {
            CandleLlmError::Execution {
                alias: self.alias.clone(),
                detail: format!("failed to decode generated tokens: {error}"),
            }
        })?;
        Ok(text.into_bytes())
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
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

    fn next_token_id(&self) -> Result<u32, CandleLlmError> {
        self.next_token_logits
            .argmax(0)
            .and_then(|id| id.to_vec0::<u32>())
            .map_err(|error| CandleLlmError::Execution {
                alias: self.alias.clone(),
                detail: format!("failed to sample greedy token with Candle: {error}"),
            })
    }
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

fn load_logits(
    alias: &str,
    root: &Path,
    config: &CandleLlmConfig,
    mut tensors: HashMap<String, Tensor>,
) -> Result<Tensor, CandleLlmError> {
    let logits =
        tensors
            .remove(NEXT_TOKEN_LOGITS)
            .ok_or_else(|| CandleLlmError::InvalidComponent {
                alias: alias.to_owned(),
                path: root.to_path_buf(),
                component: MODEL_SAFETENSORS,
                detail: format!("missing tensor `{NEXT_TOKEN_LOGITS}`"),
            })?;
    if logits.dtype() != DType::F32 {
        return Err(CandleLlmError::InvalidComponent {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            component: MODEL_SAFETENSORS,
            detail: format!(
                "tensor `{NEXT_TOKEN_LOGITS}` must use F32, got {:?}",
                logits.dtype()
            ),
        });
    }
    if logits.dims() != [config.vocab_size] {
        return Err(CandleLlmError::InvalidComponent {
            alias: alias.to_owned(),
            path: root.to_path_buf(),
            component: MODEL_SAFETENSORS,
            detail: format!(
                "tensor `{NEXT_TOKEN_LOGITS}` must have shape [{}], got {:?}",
                config.vocab_size,
                logits.dims()
            ),
        });
    }
    Ok(logits)
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
pub(crate) fn write_tachyon_tiny_fixture(root: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(root)?;
    fs::write(
        root.join(CONFIG_JSON),
        serde_json::json!({
            "model_type": TACHYON_TINY_MODEL_TYPE,
            "architectures": [TACHYON_TINY_ARCHITECTURE],
            "vocab_size": 4,
            "max_position_embeddings": 16,
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
    let logits = Tensor::from_vec(vec![0.0f32, 0.1, 4.0, 0.2], (4,), &Device::Cpu)?;
    let mut tensors = HashMap::new();
    tensors.insert(NEXT_TOKEN_LOGITS.to_owned(), logits);
    safetensors::save(&tensors, root.join(MODEL_SAFETENSORS))?;
    Ok(())
}
