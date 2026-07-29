use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{anyhow, bail, Context, Result};
use candle_core::{Device, Tensor};
use candle_transformers::generation::{LogitsProcessor, Sampling};
use serde::Deserialize;
use serde_json::Value;
use tokenizers::Tokenizer;

use super::{
    architecture_registry::{ArchitectureDescriptor, ArchitectureKind, ArchitectureMatch},
    candle_llm_runtime::{ChatTemplate, ChatTurn, TokenUsage, HOST_MAX_NEW_TOKENS},
    modelopt_nvfp4::{
        ModelOptLinearTensors, ModelOptNvfp4Directory, Nvfp4AcceleratorCapabilities,
        Nvfp4ExecutionPlan, Nvfp4FallbackMemoryLimits, Nvfp4FallbackScope,
        Nvfp4KernelSelectionMode, Nvfp4OutputDType, PreparedLinear, SafetensorsDType,
        NVFP4_BLOCK_SIZE,
    },
};
use primitives::{
    full_attention_step, gated_delta_step, rms_norm_qwen, sparse_moe_forward, HybridDecodeState,
    LayerDecodeState,
};

#[path = "qwen35_moe_primitives.rs"]
mod primitives;

pub(crate) const QWEN35_MOE_PROFILE_V1: &str = "qwen3.5-moe-text-modelopt-0.44-v1";
const OUTER_MODEL_TYPE: &str = "qwen3_5_moe";
const TEXT_MODEL_TYPE: &str = "qwen3_5_moe_text";
const ARCHITECTURE: &str = "Qwen3_5MoeForConditionalGeneration";
const ACCEPTED_PRODUCER: &str = "modelopt";
const ACCEPTED_PRODUCER_VERSION: &str = "0.44.0";

pub(crate) static QWEN35_MOE_DESCRIPTOR: Qwen35MoeDescriptor = Qwen35MoeDescriptor;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Qwen35MoeDescriptor;

impl ArchitectureDescriptor for Qwen35MoeDescriptor {
    fn inspect(&self, model: &ModelOptNvfp4Directory) -> Result<Option<ArchitectureMatch>> {
        let config = model
            .config_json()
            .context("ModelOpt Qwen 3.5 profile requires config.json")?;
        if !declares_qwen35(config) {
            return Ok(None);
        }
        Qwen35MoeConfig::validate_model(model)?;
        Ok(Some(ArchitectureMatch {
            kind: ArchitectureKind::Qwen35MoeText,
            profile: QWEN35_MOE_PROFILE_V1,
        }))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LayerType {
    LinearAttention,
    FullAttention,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Qwen35MoeConfig {
    pub(crate) model_type: String,
    pub(crate) hidden_size: usize,
    pub(crate) vocab_size: usize,
    pub(crate) num_hidden_layers: usize,
    pub(crate) layer_types: Vec<LayerType>,
    pub(crate) num_attention_heads: usize,
    pub(crate) num_key_value_heads: usize,
    pub(crate) head_dim: usize,
    pub(crate) partial_rotary_factor: f64,
    pub(crate) rope_parameters: RopeParameters,
    pub(crate) rms_norm_eps: f64,
    pub(crate) linear_conv_kernel_dim: usize,
    pub(crate) linear_key_head_dim: usize,
    pub(crate) linear_value_head_dim: usize,
    pub(crate) linear_num_key_heads: usize,
    pub(crate) linear_num_value_heads: usize,
    pub(crate) num_experts: usize,
    pub(crate) num_experts_per_tok: usize,
    pub(crate) moe_intermediate_size: usize,
    pub(crate) shared_expert_intermediate_size: usize,
    #[serde(default)]
    pub(crate) mtp_num_hidden_layers: usize,
    pub(crate) eos_token_id: u32,
    pub(crate) max_position_embeddings: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct RopeParameters {
    pub(crate) rope_theta: f64,
}

impl Qwen35MoeConfig {
    pub(crate) fn from_model(model: &ModelOptNvfp4Directory) -> Result<Self> {
        let root = model
            .config_json()
            .context("Qwen 3.5 MoE model is missing config.json")?;
        let text = root
            .get("text_config")
            .context("Qwen 3.5 MoE config is missing text_config")?;
        serde_json::from_value(text.clone()).context("invalid Qwen 3.5 MoE text_config")
    }

    pub(crate) fn validate_model(model: &ModelOptNvfp4Directory) -> Result<Self> {
        validate_producer(model)?;
        validate_quantization_metadata(model)?;
        let config = Self::from_model(model)?;
        config.validate_semantics()?;
        config.validate_tensor_contract(model)?;
        Ok(config)
    }

    fn validate_semantics(&self) -> Result<()> {
        if self.model_type != TEXT_MODEL_TYPE {
            bail!(
                "Qwen 3.5 compatibility profile expects text model_type `{TEXT_MODEL_TYPE}`, got `{}`",
                self.model_type
            );
        }
        if self.num_hidden_layers == 0 || self.layer_types.len() != self.num_hidden_layers {
            bail!(
                "Qwen 3.5 layer_types length {} does not match num_hidden_layers {}",
                self.layer_types.len(),
                self.num_hidden_layers
            );
        }
        if !self.layer_types.contains(&LayerType::LinearAttention)
            || !self.layer_types.contains(&LayerType::FullAttention)
        {
            bail!(
                "Qwen 3.5 hybrid profile requires both linear_attention and full_attention layers"
            );
        }
        if self.hidden_size == 0
            || self.vocab_size == 0
            || self.num_attention_heads == 0
            || self.num_key_value_heads == 0
            || self.head_dim == 0
        {
            bail!("Qwen 3.5 attention and embedding dimensions must be non-zero");
        }
        if !self
            .num_attention_heads
            .is_multiple_of(self.num_key_value_heads)
        {
            bail!("num_attention_heads must be divisible by num_key_value_heads");
        }
        if !(0.0..=1.0).contains(&self.partial_rotary_factor) || self.partial_rotary_factor == 0.0 {
            bail!("partial_rotary_factor must be in (0, 1]");
        }
        if self.linear_num_value_heads == 0
            || self.linear_num_key_heads == 0
            || !self
                .linear_num_value_heads
                .is_multiple_of(self.linear_num_key_heads)
            || self.linear_key_head_dim == 0
            || self.linear_value_head_dim == 0
            || self.linear_conv_kernel_dim == 0
        {
            bail!("invalid Qwen 3.5 gated-delta linear-attention dimensions");
        }
        if self.num_experts == 0
            || self.num_experts_per_tok == 0
            || self.num_experts_per_tok > self.num_experts
            || self.moe_intermediate_size == 0
            || self.shared_expert_intermediate_size == 0
        {
            bail!("invalid Qwen 3.5 sparse-MoE dimensions");
        }
        if self.rms_norm_eps <= 0.0 {
            bail!("rms_norm_eps must be positive");
        }
        let _ = self.mtp_num_hidden_layers;
        Ok(())
    }

    fn validate_tensor_contract(&self, model: &ModelOptNvfp4Directory) -> Result<()> {
        require_tensor_shape(
            model,
            "model.language_model.embed_tokens.weight",
            &[self.vocab_size, self.hidden_size],
        )?;
        require_tensor_shape(
            model,
            "model.language_model.norm.weight",
            &[self.hidden_size],
        )?;
        validate_linear_kind(
            model,
            "lm_head",
            QuantKind::Nvfp4,
            self.vocab_size,
            self.hidden_size,
        )?;

        for (layer, layer_type) in self.layer_types.iter().enumerate() {
            let prefix = format!("model.language_model.layers.{layer}");
            require_tensor_shape(
                model,
                &format!("{prefix}.input_layernorm.weight"),
                &[self.hidden_size],
            )?;
            require_tensor_shape(
                model,
                &format!("{prefix}.post_attention_layernorm.weight"),
                &[self.hidden_size],
            )?;
            match layer_type {
                LayerType::LinearAttention => {
                    let key_dim = self.linear_num_key_heads * self.linear_key_head_dim;
                    let value_dim = self.linear_num_value_heads * self.linear_value_head_dim;
                    require_tensor_shape(
                        model,
                        &format!("{prefix}.linear_attn.A_log"),
                        &[self.linear_num_value_heads],
                    )?;
                    require_tensor_shape(
                        model,
                        &format!("{prefix}.linear_attn.dt_bias"),
                        &[self.linear_num_value_heads],
                    )?;
                    require_tensor_shape(
                        model,
                        &format!("{prefix}.linear_attn.conv1d.weight"),
                        &[key_dim * 2 + value_dim, 1, self.linear_conv_kernel_dim],
                    )?;
                    for suffix in ["linear_attn.in_proj_a", "linear_attn.in_proj_b"] {
                        validate_linear_kind(
                            model,
                            &format!("{prefix}.{suffix}"),
                            QuantKind::Dense,
                            self.linear_num_value_heads,
                            self.hidden_size,
                        )?;
                    }
                    require_tensor_shape(
                        model,
                        &format!("{prefix}.linear_attn.norm.weight"),
                        &[self.linear_value_head_dim],
                    )?;
                    validate_linear_kind(
                        model,
                        &format!("{prefix}.linear_attn.in_proj_qkv"),
                        QuantKind::Fp8,
                        key_dim * 2 + value_dim,
                        self.hidden_size,
                    )?;
                    validate_linear_kind(
                        model,
                        &format!("{prefix}.linear_attn.in_proj_z"),
                        QuantKind::Fp8,
                        value_dim,
                        self.hidden_size,
                    )?;
                    validate_linear_kind(
                        model,
                        &format!("{prefix}.linear_attn.out_proj"),
                        QuantKind::Fp8,
                        self.hidden_size,
                        value_dim,
                    )?;
                }
                LayerType::FullAttention => {
                    for suffix in ["self_attn.q_norm.weight", "self_attn.k_norm.weight"] {
                        require_tensor_shape(
                            model,
                            &format!("{prefix}.{suffix}"),
                            &[self.head_dim],
                        )?;
                    }
                    validate_linear_kind(
                        model,
                        &format!("{prefix}.self_attn.q_proj"),
                        QuantKind::Fp8,
                        self.num_attention_heads * self.head_dim * 2,
                        self.hidden_size,
                    )?;
                    for suffix in ["self_attn.k_proj", "self_attn.v_proj"] {
                        validate_linear_kind(
                            model,
                            &format!("{prefix}.{suffix}"),
                            QuantKind::Fp8,
                            self.num_key_value_heads * self.head_dim,
                            self.hidden_size,
                        )?;
                    }
                    validate_linear_kind(
                        model,
                        &format!("{prefix}.self_attn.o_proj"),
                        QuantKind::Fp8,
                        self.hidden_size,
                        self.num_attention_heads * self.head_dim,
                    )?;
                }
            }

            validate_linear_kind(
                model,
                &format!("{prefix}.mlp.gate"),
                QuantKind::Dense,
                self.num_experts,
                self.hidden_size,
            )?;
            validate_linear_kind(
                model,
                &format!("{prefix}.mlp.shared_expert_gate"),
                QuantKind::Dense,
                1,
                self.hidden_size,
            )?;
            for suffix in ["gate_proj", "up_proj"] {
                validate_linear_kind(
                    model,
                    &format!("{prefix}.mlp.shared_expert.{suffix}"),
                    QuantKind::Nvfp4,
                    self.shared_expert_intermediate_size,
                    self.hidden_size,
                )?;
            }
            validate_linear_kind(
                model,
                &format!("{prefix}.mlp.shared_expert.down_proj"),
                QuantKind::Nvfp4,
                self.hidden_size,
                self.shared_expert_intermediate_size,
            )?;
            for expert in 0..self.num_experts {
                for projection in ["gate_proj", "up_proj"] {
                    validate_linear_kind(
                        model,
                        &format!("{prefix}.mlp.experts.{expert}.{projection}"),
                        QuantKind::Nvfp4,
                        self.moe_intermediate_size,
                        self.hidden_size,
                    )
                    .with_context(|| {
                        format!(
                            "invalid expert tensor contract at layer {layer}, expert {expert}, component {projection}"
                        )
                    })?;
                }
                validate_linear_kind(
                    model,
                    &format!("{prefix}.mlp.experts.{expert}.down_proj"),
                    QuantKind::Nvfp4,
                    self.hidden_size,
                    self.moe_intermediate_size,
                )
                .with_context(|| {
                    format!(
                        "invalid expert tensor contract at layer {layer}, expert {expert}, component down_proj"
                    )
                })?;
            }
        }
        Ok(())
    }
}

pub(crate) struct Qwen35MoeRuntime {
    alias: String,
    root: PathBuf,
    model: ModelOptNvfp4Directory,
    config: Qwen35MoeConfig,
    tokenizer: Tokenizer,
    chat_template: Option<ChatTemplate>,
    max_dense_operator_bytes: u64,
    execution_plan: Nvfp4ExecutionPlan,
    executed_on: &'static str,
    working_set: Mutex<LinearWorkingSet>,
}

#[derive(Debug, Default)]
struct WorkingSetStats {
    hits: u64,
    misses: u64,
    evictions: u64,
    transfers: u64,
}

#[derive(Debug)]
struct LinearWorkingSet {
    max_bytes: u64,
    resident_bytes: u64,
    values: BTreeMap<String, Arc<PreparedLinear>>,
    order: VecDeque<String>,
    stats: WorkingSetStats,
}

impl LinearWorkingSet {
    fn new(max_bytes: u64) -> Self {
        Self {
            max_bytes,
            resident_bytes: 0,
            values: BTreeMap::new(),
            order: VecDeque::new(),
            stats: WorkingSetStats::default(),
        }
    }

    fn get(&mut self, key: &str) -> Option<Arc<PreparedLinear>> {
        let value = self.values.get(key).cloned();
        if value.is_some() {
            self.stats.hits += 1;
            self.order.retain(|entry| entry != key);
            self.order.push_back(key.to_owned());
        } else {
            self.stats.misses += 1;
        }
        value
    }

    fn insert(&mut self, key: String, value: Arc<PreparedLinear>) {
        let bytes = value.resident_bytes();
        self.stats.transfers += 1;
        if bytes > self.max_bytes {
            return;
        }
        while self.resident_bytes.saturating_add(bytes) > self.max_bytes {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(evicted) = self.values.remove(&oldest) {
                self.resident_bytes = self.resident_bytes.saturating_sub(evicted.resident_bytes());
                self.stats.evictions += 1;
            }
        }
        self.resident_bytes = self.resident_bytes.saturating_add(bytes);
        self.order.push_back(key.clone());
        self.values.insert(key, value);
    }
}

#[derive(Debug, Deserialize)]
struct GenerationRequest {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    messages: Option<Vec<IncomingChatTurn>>,
    #[serde(default = "default_max_new_tokens")]
    max_new_tokens: usize,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    stop: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct IncomingChatTurn {
    role: String,
    content: Value,
}

fn default_max_new_tokens() -> usize {
    64
}

impl Qwen35MoeRuntime {
    pub(crate) fn try_load(
        alias: &str,
        path: impl AsRef<Path>,
        requested_device: &str,
    ) -> Result<Option<Self>> {
        let Some(model) = ModelOptNvfp4Directory::try_load(alias, path.as_ref())? else {
            return Ok(None);
        };
        Self::from_model(alias, path, requested_device, model).map(Some)
    }

    pub(crate) fn from_model(
        alias: &str,
        path: impl AsRef<Path>,
        requested_device: &str,
        model: ModelOptNvfp4Directory,
    ) -> Result<Self> {
        let matched = QWEN35_MOE_DESCRIPTOR.inspect(&model)?.ok_or_else(|| {
            anyhow!("ModelOpt/NVFP4 model `{alias}` does not match the Qwen 3.5 MoE text profile")
        })?;
        if matched.kind != ArchitectureKind::Qwen35MoeText {
            bail!("resolved architecture is not Qwen 3.5 MoE text");
        }
        if requested_device != "cpu" && requested_device != "gpu" {
            bail!(
                "Qwen 3.5 MoE runtime supports `cpu` or capability-gated `gpu`, got `{requested_device}`"
            );
        }
        let root = path.as_ref();
        let tokenizer = Tokenizer::from_file(root.join("tokenizer.json"))
            .map_err(|error| anyhow!("failed to load Qwen tokenizer.json: {error}"))?;
        let chat_template = ChatTemplate::load(alias, root)?;
        let config = Qwen35MoeConfig::validate_model(&model)?;
        let max_dense_operator_bytes = std::env::var("TACHYON_QWEN35_MAX_DENSE_OPERATOR_BYTES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(512 * 1024 * 1024);
        let working_set_bytes = std::env::var("TACHYON_QWEN35_WORKING_SET_BYTES")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1024 * 1024 * 1024);
        let fallback_limits = Nvfp4FallbackMemoryLimits {
            max_host_ram_bytes: env_u64("TACHYON_NVFP4_MAX_HOST_RAM_BYTES"),
            max_accelerator_bytes: env_u64("TACHYON_NVFP4_MAX_ACCELERATOR_BYTES"),
        };
        let mode = if std::env::var("TACHYON_NVFP4_NATIVE_REQUIRED").as_deref() == Ok("1") {
            Nvfp4KernelSelectionMode::NativeRequired
        } else {
            Nvfp4KernelSelectionMode::NativePreferred
        };
        let capabilities = if requested_device == "gpu" {
            Nvfp4AcceleratorCapabilities::cuda_cutlass()
        } else {
            Nvfp4AcceleratorCapabilities::fallback_only()
        };
        let execution_plan = model.select_execution_plan(
            &capabilities,
            Nvfp4OutputDType::F32,
            Nvfp4FallbackScope::LayerWindow(1),
            fallback_limits,
            mode,
        )?;
        let executed_on = match (&execution_plan, requested_device) {
            (Nvfp4ExecutionPlan::Native { .. }, _) => "gpu_native_fp4",
            (Nvfp4ExecutionPlan::Fallback(_), "gpu") => "gpu_fallback",
            (Nvfp4ExecutionPlan::Fallback(_), _) => "cpu_fallback",
        };
        Ok(Self {
            alias: alias.to_owned(),
            root: root.to_path_buf(),
            model,
            config,
            tokenizer,
            chat_template,
            max_dense_operator_bytes,
            execution_plan,
            executed_on,
            working_set: Mutex::new(LinearWorkingSet::new(working_set_bytes)),
        })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn executed_on(&self) -> &'static str {
        self.executed_on
    }

    pub(crate) fn generate(&self, prompts: &[&[u8]]) -> Result<Vec<u8>> {
        let mut output = String::new();
        self.generate_streaming(prompts, &mut |delta| output.push_str(delta))?;
        Ok(output.into_bytes())
    }

    pub(crate) fn generate_streaming(
        &self,
        prompts: &[&[u8]],
        on_token: &mut dyn FnMut(&str),
    ) -> Result<TokenUsage> {
        if prompts.len() != 1 {
            bail!("Qwen 3.5 MoE runtime currently accepts exactly one prompt per decode");
        }
        let request = self.parse_request(prompts[0])?;
        let encoded = self
            .tokenizer
            .encode(request.prompt.clone(), true)
            .map_err(|error| anyhow!("failed to tokenize Qwen prompt: {error}"))?;
        if encoded.is_empty() {
            bail!("Qwen prompt encoded to zero tokens");
        }
        if encoded.len() + request.max_new_tokens > self.config.max_position_embeddings {
            bail!(
                "prompt and generation length exceed context limit {}",
                self.config.max_position_embeddings
            );
        }

        let mut state = HybridDecodeState::new(&self.config.layer_types);
        let mut logits = Vec::new();
        for (position, token) in encoded.get_ids().iter().copied().enumerate() {
            logits = self.forward_token(token, position, &mut state)?;
        }
        let sampling = resolve_sampling(
            request.temperature,
            request.top_p,
            request.seed.unwrap_or(299_792_458),
        );
        let mut processor =
            LogitsProcessor::from_sampling(request.seed.unwrap_or(299_792_458), sampling);
        let mut generated = Vec::<u32>::new();
        let mut emitted = 0usize;
        for step in 0..request.max_new_tokens {
            let tensor = Tensor::from_vec(logits, self.config.vocab_size, &Device::Cpu)?;
            let token = processor.sample(&tensor)?;
            if token == self.config.eos_token_id {
                break;
            }
            generated.push(token);
            let text = self
                .tokenizer
                .decode(&generated, true)
                .map_err(|error| anyhow!("failed to decode Qwen tokens: {error}"))?;
            let stop = request
                .stop
                .iter()
                .filter_map(|needle| text.find(needle))
                .min();
            let safe_end = stop.unwrap_or(text.len());
            if emitted < safe_end {
                on_token(&text[emitted..safe_end]);
                emitted = safe_end;
            }
            if stop.is_some() {
                break;
            }
            logits = self.forward_token(token, encoded.len() + step, &mut state)?;
        }
        Ok(TokenUsage {
            prompt_tokens: encoded.len() as u32,
            completion_tokens: generated.len() as u32,
        })
    }

    fn parse_request(&self, bytes: &[u8]) -> Result<ParsedRequest> {
        if bytes.len() > 16_384 {
            bail!("Qwen generation request exceeds 16384 bytes");
        }
        let raw = std::str::from_utf8(bytes).context("Qwen prompt must be UTF-8")?;
        let request = if raw.trim_start().starts_with('{') {
            serde_json::from_str::<GenerationRequest>(raw)
                .context("invalid Qwen JSON generation request")?
        } else {
            GenerationRequest {
                prompt: Some(raw.to_owned()),
                messages: None,
                max_new_tokens: default_max_new_tokens(),
                temperature: None,
                top_p: None,
                seed: None,
                stop: Vec::new(),
            }
        };
        if request.max_new_tokens == 0 || request.max_new_tokens > HOST_MAX_NEW_TOKENS {
            bail!("max_new_tokens must be between 1 and {HOST_MAX_NEW_TOKENS}");
        }
        let prompt = match (request.messages, request.prompt) {
            (Some(messages), _) if messages.is_empty() => {
                bail!("chat request must contain at least one message")
            }
            (Some(messages), _) => {
                let messages = messages
                    .into_iter()
                    .map(|message| {
                        let content = message.content.as_str().ok_or_else(|| {
                            anyhow!(
                                "Qwen 3.5 text runtime does not support image or structured message content"
                            )
                        })?;
                        Ok(ChatTurn {
                            role: message.role,
                            content: content.to_owned(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                match &self.chat_template {
                    Some(template) => template
                        .render(&messages)
                        .map_err(|error| anyhow!("failed to render Qwen chat template: {error}"))?,
                    None => {
                        let mut prompt = String::new();
                        for message in messages {
                            prompt.push_str(&message.role);
                            prompt.push_str(": ");
                            prompt.push_str(&message.content);
                            prompt.push('\n');
                        }
                        prompt.push_str("assistant:");
                        prompt
                    }
                }
            }
            (None, Some(prompt)) => prompt,
            (None, None) => bail!("generation request requires `messages` or `prompt`"),
        };
        Ok(ParsedRequest {
            prompt,
            max_new_tokens: request.max_new_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
            seed: request.seed,
            stop: request
                .stop
                .into_iter()
                .filter(|stop| !stop.is_empty() && stop.len() <= 256)
                .take(8)
                .collect(),
        })
    }

    fn forward_token(
        &self,
        token: u32,
        position: usize,
        state: &mut HybridDecodeState,
    ) -> Result<Vec<f32>> {
        let hidden_size = self.config.hidden_size;
        let embedding = self
            .model
            .tensor_info("model.language_model.embed_tokens.weight")?;
        let token = usize::try_from(token)?;
        if token >= self.config.vocab_size {
            bail!("token id {token} exceeds vocabulary");
        }
        let mut hidden = embedding.read_f32_slice(token * hidden_size, hidden_size)?;
        for layer in 0..self.config.num_hidden_layers {
            let prefix = format!("model.language_model.layers.{layer}");
            let norm_weight = self
                .model
                .tensor_info(&format!("{prefix}.input_layernorm.weight"))?
                .read_f32()?;
            let normalized = rms_norm_qwen(&hidden, &norm_weight, self.config.rms_norm_eps as f32)?;
            let mixed = match (&self.config.layer_types[layer], &mut state.layers[layer]) {
                (LayerType::LinearAttention, LayerDecodeState::Linear(layer_state)) => {
                    let qkv =
                        self.linear(&format!("{prefix}.linear_attn.in_proj_qkv"), &normalized)?;
                    let z = self.linear(&format!("{prefix}.linear_attn.in_proj_z"), &normalized)?;
                    let b = self.linear(&format!("{prefix}.linear_attn.in_proj_b"), &normalized)?;
                    let a = self.linear(&format!("{prefix}.linear_attn.in_proj_a"), &normalized)?;
                    let conv = self
                        .model
                        .tensor_info(&format!("{prefix}.linear_attn.conv1d.weight"))?
                        .read_f32()?;
                    let a_log = self
                        .model
                        .tensor_info(&format!("{prefix}.linear_attn.A_log"))?
                        .read_f32()?;
                    let dt_bias = self
                        .model
                        .tensor_info(&format!("{prefix}.linear_attn.dt_bias"))?
                        .read_f32()?;
                    let gated_norm = self
                        .model
                        .tensor_info(&format!("{prefix}.linear_attn.norm.weight"))?
                        .read_f32()?;
                    let core = gated_delta_step(
                        &qkv,
                        &z,
                        &b,
                        &a,
                        &conv,
                        &a_log,
                        &dt_bias,
                        &gated_norm,
                        self.config.linear_num_key_heads,
                        self.config.linear_num_value_heads,
                        self.config.linear_key_head_dim,
                        self.config.linear_value_head_dim,
                        self.config.linear_conv_kernel_dim,
                        layer_state,
                    )?;
                    self.linear(&format!("{prefix}.linear_attn.out_proj"), &core)?
                }
                (LayerType::FullAttention, LayerDecodeState::Full(layer_state)) => {
                    let query = self.linear(&format!("{prefix}.self_attn.q_proj"), &normalized)?;
                    let key = self.linear(&format!("{prefix}.self_attn.k_proj"), &normalized)?;
                    let value = self.linear(&format!("{prefix}.self_attn.v_proj"), &normalized)?;
                    let q_norm = self
                        .model
                        .tensor_info(&format!("{prefix}.self_attn.q_norm.weight"))?
                        .read_f32()?;
                    let k_norm = self
                        .model
                        .tensor_info(&format!("{prefix}.self_attn.k_norm.weight"))?
                        .read_f32()?;
                    let core = full_attention_step(
                        &query,
                        &key,
                        &value,
                        &q_norm,
                        &k_norm,
                        self.config.num_attention_heads,
                        self.config.num_key_value_heads,
                        self.config.head_dim,
                        (self.config.head_dim as f64 * self.config.partial_rotary_factor) as usize,
                        self.config.rope_parameters.rope_theta as f32,
                        position,
                        layer_state,
                    )?;
                    self.linear(&format!("{prefix}.self_attn.o_proj"), &core)?
                }
                _ => bail!("hybrid decode state does not match layer {layer}"),
            };
            add_assign(&mut hidden, &mixed)?;
            let post_norm_weight = self
                .model
                .tensor_info(&format!("{prefix}.post_attention_layernorm.weight"))?
                .read_f32()?;
            let post_norm =
                rms_norm_qwen(&hidden, &post_norm_weight, self.config.rms_norm_eps as f32)?;
            let router = self.linear(&format!("{prefix}.mlp.gate"), &post_norm)?;
            let shared_gate =
                self.linear(&format!("{prefix}.mlp.shared_expert_gate"), &post_norm)?[0];
            let moe = sparse_moe_forward(
                &post_norm,
                &router,
                self.config.num_experts_per_tok,
                |expert, input| self.mlp(&format!("{prefix}.mlp.experts.{expert}"), input),
                |input| self.mlp(&format!("{prefix}.mlp.shared_expert"), input),
                shared_gate,
            )?;
            add_assign(&mut hidden, &moe)?;
        }
        let final_norm = self
            .model
            .tensor_info("model.language_model.norm.weight")?
            .read_f32()?;
        let hidden = rms_norm_qwen(&hidden, &final_norm, self.config.rms_norm_eps as f32)?;
        self.linear("lm_head", &hidden)
    }

    fn mlp(&self, prefix: &str, input: &[f32]) -> Result<Vec<f32>> {
        let gate = self.linear(&format!("{prefix}.gate_proj"), input)?;
        let up = self.linear(&format!("{prefix}.up_proj"), input)?;
        let activated = gate
            .into_iter()
            .zip(up)
            .map(|(gate, up)| primitives::silu(gate) * up)
            .collect::<Vec<_>>();
        self.linear(&format!("{prefix}.down_proj"), &activated)
    }

    fn linear(&self, base: &str, input: &[f32]) -> Result<Vec<f32>> {
        let cached = self
            .working_set
            .lock()
            .map_err(|_| anyhow!("Qwen working-set lock is poisoned"))?
            .get(base);
        let prepared = match cached {
            Some(prepared) => prepared,
            None => {
                let prepared = Arc::new(
                    self.model
                        .modelopt_linear(base)?
                        .prepare(Some(self.max_dense_operator_bytes))?,
                );
                self.working_set
                    .lock()
                    .map_err(|_| anyhow!("Qwen working-set lock is poisoned"))?
                    .insert(base.to_owned(), prepared.clone());
                prepared
            }
        };
        match &self.execution_plan {
            Nvfp4ExecutionPlan::Native { .. } => prepared.matvec_native_nvfp4(input),
            Nvfp4ExecutionPlan::Fallback(_) => prepared.matvec(input),
        }
        .with_context(|| format!("Qwen operator `{base}` failed"))
    }
}

fn env_u64(name: &str) -> Option<u64> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
}

#[derive(Debug)]
struct ParsedRequest {
    prompt: String,
    max_new_tokens: usize,
    temperature: Option<f32>,
    top_p: Option<f32>,
    seed: Option<u64>,
    stop: Vec<String>,
}

fn resolve_sampling(temperature: Option<f32>, top_p: Option<f32>, _seed: u64) -> Sampling {
    let Some(temperature) = temperature.filter(|value| value.is_finite() && *value > 1e-7) else {
        return Sampling::ArgMax;
    };
    match top_p.filter(|value| value.is_finite() && *value > 0.0 && *value < 1.0) {
        Some(p) => Sampling::TopP {
            p: f64::from(p),
            temperature: f64::from(temperature),
        },
        None => Sampling::All {
            temperature: f64::from(temperature),
        },
    }
}

fn add_assign(left: &mut [f32], right: &[f32]) -> Result<()> {
    if left.len() != right.len() {
        bail!(
            "residual dimensions differ: {} and {}",
            left.len(),
            right.len()
        );
    }
    for (left, right) in left.iter_mut().zip(right) {
        *left += right;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum QuantKind {
    Dense,
    Fp8,
    Nvfp4,
}

fn declares_qwen35(config: &Value) -> bool {
    let architecture_matches = config
        .get("architectures")
        .and_then(Value::as_array)
        .is_some_and(|architectures| {
            architectures
                .iter()
                .any(|value| value.as_str() == Some(ARCHITECTURE))
        });
    let outer_matches = config.get("model_type").and_then(Value::as_str) == Some(OUTER_MODEL_TYPE);
    let text_matches = config
        .get("text_config")
        .and_then(|text| text.get("model_type"))
        .and_then(Value::as_str)
        == Some(TEXT_MODEL_TYPE);
    architecture_matches && outer_matches && text_matches
}

fn validate_producer(model: &ModelOptNvfp4Directory) -> Result<()> {
    let producer = model
        .hf_quant_json()
        .and_then(|value| value.get("producer"))
        .context("Qwen 3.5 ModelOpt profile requires hf_quant_config.json producer metadata")?;
    let name = producer.get("name").and_then(Value::as_str).unwrap_or("");
    let version = producer
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("");
    if name != ACCEPTED_PRODUCER || version != ACCEPTED_PRODUCER_VERSION {
        bail!(
            "compatibility profile `{QWEN35_MOE_PROFILE_V1}` accepts producer {ACCEPTED_PRODUCER} {ACCEPTED_PRODUCER_VERSION}, got `{name}` `{version}`"
        );
    }
    Ok(())
}

fn validate_quantization_metadata(model: &ModelOptNvfp4Directory) -> Result<()> {
    let quantization = model
        .hf_quant_json()
        .and_then(|value| value.get("quantization"))
        .context("Qwen 3.5 ModelOpt profile requires quantization metadata")?;
    let algorithm = quantization
        .get("quant_algo")
        .and_then(Value::as_str)
        .unwrap_or("");
    let kv_algorithm = quantization
        .get("kv_cache_quant_algo")
        .and_then(Value::as_str)
        .unwrap_or("");
    if algorithm != "MIXED_PRECISION" || kv_algorithm != "FP8" {
        bail!(
            "expected MIXED_PRECISION weights with FP8 KV cache, got `{algorithm}` and `{kv_algorithm}`"
        );
    }
    let layers = quantization
        .get("quantized_layers")
        .and_then(Value::as_object)
        .context("quantization.quantized_layers must be an object")?;
    let supported = BTreeSet::from(["FP8", "W4A16_NVFP4"]);
    let mut seen = BTreeMap::<&str, usize>::new();
    for (name, metadata) in layers {
        let algo = metadata
            .get("quant_algo")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("quantized layer `{name}` is missing quant_algo"))?;
        if !supported.contains(algo) {
            bail!("quantized layer `{name}` uses unsupported algorithm `{algo}`");
        }
        if algo == "W4A16_NVFP4"
            && metadata.get("group_size").and_then(Value::as_u64) != Some(NVFP4_BLOCK_SIZE as u64)
        {
            bail!("quantized layer `{name}` must use NVFP4 group size {NVFP4_BLOCK_SIZE}");
        }
        *seen.entry(algo).or_default() += 1;
    }
    if !seen.contains_key("FP8") || !seen.contains_key("W4A16_NVFP4") {
        bail!("mixed-precision profile requires both FP8 and W4A16_NVFP4 operators");
    }
    Ok(())
}

fn require_tensor(model: &ModelOptNvfp4Directory, name: &str) -> Result<()> {
    if model.contains_tensor(name) {
        Ok(())
    } else {
        bail!("required Qwen 3.5 tensor `{name}` is missing")
    }
}

fn require_tensor_shape(
    model: &ModelOptNvfp4Directory,
    name: &str,
    expected: &[usize],
) -> Result<()> {
    require_tensor(model, name)?;
    let tensor = model.tensor_info(name)?;
    if tensor.info.shape != expected {
        bail!(
            "tensor `{name}` has shape {:?}, expected {expected:?}",
            tensor.info.shape
        );
    }
    Ok(())
}

fn validate_linear_kind(
    model: &ModelOptNvfp4Directory,
    base: &str,
    expected: QuantKind,
    out_dim: usize,
    in_dim: usize,
) -> Result<()> {
    let (actual, shape) = match model.modelopt_linear(base)? {
        ModelOptLinearTensors::Fp8(linear) => {
            if linear.weight.info.dtype != SafetensorsDType::F8E4M3 {
                bail!("FP8 operator `{base}` weight must use F8_E4M3");
            }
            if linear
                .weight_scale
                .tensor
                .info
                .shape
                .iter()
                .product::<usize>()
                != 1
            {
                bail!("FP8 operator `{base}` weight_scale must be scalar");
            }
            if linear.weight_scale.tensor.info.dtype != SafetensorsDType::F32 {
                bail!("FP8 operator `{base}` weight_scale must use F32");
            }
            if linear.input_scale.as_ref().is_some_and(|scale| {
                scale.tensor.info.shape.iter().product::<usize>() != 1
                    || scale.tensor.info.dtype != SafetensorsDType::F32
            }) {
                bail!("FP8 operator `{base}` input_scale must be an F32 scalar");
            }
            (QuantKind::Fp8, linear.weight.info.shape)
        }
        ModelOptLinearTensors::Nvfp4(linear) => {
            if linear.packed_weight.tensor.info.dtype != SafetensorsDType::U8
                || linear.block_scales.tensor.info.dtype != SafetensorsDType::F8E4M3
                || linear.tensor_scale.tensor.info.dtype != SafetensorsDType::F32
            {
                bail!("NVFP4 operator `{base}` has invalid packed or scale dtypes");
            }
            let packed = &linear.packed_weight.tensor.info.shape;
            (QuantKind::Nvfp4, vec![packed[0], packed[1] * 2])
        }
        ModelOptLinearTensors::Passthrough(linear) => (QuantKind::Dense, linear.tensor.info.shape),
    };
    if actual != expected {
        bail!("operator `{base}` uses {actual:?}, expected {expected:?}");
    }
    let expected_shape = vec![out_dim, in_dim];
    if shape != expected_shape {
        bail!("operator `{base}` has shape {shape:?}, expected {expected_shape:?}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architecture_detection_uses_metadata_not_directory_name() {
        let config = serde_json::json!({
            "architectures": [ARCHITECTURE],
            "model_type": OUTER_MODEL_TYPE,
            "text_config": {"model_type": TEXT_MODEL_TYPE}
        });
        assert!(declares_qwen35(&config));
    }

    #[test]
    fn architecture_detection_fails_closed_for_semantic_variants() {
        let config = serde_json::json!({
            "architectures": ["OtherArchitecture"],
            "model_type": OUTER_MODEL_TYPE,
            "text_config": {"model_type": TEXT_MODEL_TYPE}
        });
        assert!(!declares_qwen35(&config));
    }

    #[test]
    fn gated_installed_checkpoint_validates_complete_profile() {
        let Ok(path) = std::env::var("TACHYON_QWEN35_MOE_NVFP4_DIR") else {
            return;
        };
        let model = ModelOptNvfp4Directory::try_load("qwen35-probe", path)
            .expect("checkpoint parser should succeed")
            .expect("checkpoint should be detected as ModelOpt/NVFP4");
        let matched = QWEN35_MOE_DESCRIPTOR
            .inspect(&model)
            .expect("installed checkpoint should match")
            .expect("installed checkpoint should select Qwen 3.5");
        assert_eq!(matched.kind, ArchitectureKind::Qwen35MoeText);
        assert_eq!(matched.profile, QWEN35_MOE_PROFILE_V1);
    }

    #[test]
    fn gated_installed_checkpoint_constructs_text_runtime() {
        let Ok(path) = std::env::var("TACHYON_QWEN35_MOE_NVFP4_DIR") else {
            return;
        };
        let runtime = Qwen35MoeRuntime::try_load("qwen35-probe", &path, "cpu")
            .expect("runtime construction should succeed")
            .expect("checkpoint should select the Qwen runtime");
        assert_eq!(runtime.config.num_hidden_layers, 40);
        assert_eq!(runtime.config.num_experts, 256);
        assert!(runtime.chat_template.is_some());
    }

    #[test]
    fn working_set_is_bounded_and_reports_cache_activity() {
        let mut cache = LinearWorkingSet::new(8);
        let first = Arc::new(PreparedLinear::Fp8 {
            rows: 1,
            cols: 4,
            weight: vec![0; 4],
            scale: 1.0,
        });
        let second = Arc::new(PreparedLinear::Fp8 {
            rows: 1,
            cols: 8,
            weight: vec![0; 8],
            scale: 1.0,
        });
        cache.insert("first".to_owned(), first);
        assert!(cache.get("first").is_some());
        cache.insert("second".to_owned(), second);
        assert!(cache.resident_bytes <= cache.max_bytes);
        assert_eq!(cache.stats.hits, 1);
        assert!(cache.stats.transfers >= 2);
        assert!(cache.stats.evictions >= 1);
    }

    #[test]
    fn gated_installed_checkpoint_generates_buffered_and_streaming_text() {
        if std::env::var("TACHYON_QWEN35_RUN_SLOW_PROBE").as_deref() != Ok("1") {
            return;
        }
        let path = std::env::var("TACHYON_QWEN35_MOE_NVFP4_DIR")
            .expect("slow probe requires TACHYON_QWEN35_MOE_NVFP4_DIR");
        let runtime = Qwen35MoeRuntime::try_load("qwen35-probe", path, "cpu")
            .expect("runtime construction")
            .expect("Qwen runtime");
        let request = br#"{"prompt":"Hi","max_new_tokens":1,"temperature":0}"#;

        let host_memory_before = current_process_memory_bytes();
        let gpu_memory_before = nvidia_memory_used_mib();
        let started = std::time::Instant::now();
        let buffered = runtime.generate(&[request]).expect("buffered generation");
        let first_token_ms = started.elapsed().as_millis();
        let mut streamed = String::new();
        let decode_started = std::time::Instant::now();
        runtime
            .generate_streaming(&[request], &mut |delta| streamed.push_str(delta))
            .expect("streaming generation");
        let decode_ms = decode_started.elapsed().as_millis();

        assert!(!buffered.is_empty());
        assert_ne!(buffered, b"MOCK_LLM_RESPONSE");
        assert_eq!(String::from_utf8(buffered).expect("UTF-8"), streamed);
        let working_set = runtime.working_set.lock().expect("working set");
        eprintln!(
            "{}",
            serde_json::json!({
                "first_token_ms": first_token_ms,
                "streaming_decode_ms": decode_ms,
                "state_profile": "layer-wise-streaming",
                "working_set_resident_bytes": working_set.resident_bytes,
                "working_set_hits": working_set.stats.hits,
                "working_set_misses": working_set.stats.misses,
                "working_set_transfers": working_set.stats.transfers,
                "working_set_evictions": working_set.stats.evictions,
                "host_memory_before_bytes": host_memory_before,
                "host_memory_after_bytes": current_process_memory_bytes(),
                "gpu_memory_before_mib": gpu_memory_before,
                "gpu_memory_after_mib": nvidia_memory_used_mib(),
            })
        );
    }

    fn current_process_memory_bytes() -> Option<u64> {
        let mut system = sysinfo::System::new();
        system.refresh_processes(
            sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(std::process::id())]),
            true,
        );
        system
            .process(sysinfo::Pid::from_u32(std::process::id()))
            .map(sysinfo::Process::memory)
    }

    fn nvidia_memory_used_mib() -> Option<u64> {
        let output = std::process::Command::new("nvidia-smi")
            .args(["--query-gpu=memory.used", "--format=csv,noheader,nounits"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        String::from_utf8(output.stdout)
            .ok()?
            .lines()
            .next()?
            .trim()
            .parse()
            .ok()
    }
}
