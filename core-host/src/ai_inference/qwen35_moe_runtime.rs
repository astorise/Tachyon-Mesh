use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{anyhow, bail, Context, Result};
use candle_core::{Device, Tensor};
use candle_transformers::generation::{
    FinishReason, IncrementalDecoder, LogitsProcessor, Sampling, StopCriteria,
};
use serde::Deserialize;
use serde_json::Value;
use tokenizers::Tokenizer;

use super::{
    architecture_registry::{ArchitectureDescriptor, ArchitectureKind, ArchitectureMatch},
    candle_llm_runtime::{
        prompt_limits_for, ChatTemplate, ChatTurn, TokenUsage, DEFAULT_GENERATION_DEADLINE,
        HOST_MAX_GENERATION_DEADLINE, HOST_MAX_NEW_TOKENS,
    },
    modelopt_nvfp4::{
        dequantized_fallback_opted_in, ModelOptLinearTensors, ModelOptNvfp4Directory,
        Nvfp4ExecutionPlan, Nvfp4FallbackMemoryLimits, Nvfp4FallbackPolicy, Nvfp4FallbackScope,
        Nvfp4KernelAvailability, Nvfp4OutputDType, PreparedLinear, SafetensorsDType,
        NVFP4_BLOCK_SIZE,
    },
    StreamControl,
};
use primitives::{
    full_attention_step, gated_delta_step, rms_norm_qwen, sparse_moe_forward_batch,
    HybridDecodeState, LayerDecodeState,
};

#[path = "qwen35_moe_primitives.rs"]
mod primitives;

pub(crate) const QWEN35_MOE_PROFILE_V1: &str = "qwen3.5-moe-text-modelopt-0.44-v1";
/// Prompt tokens per prefill pass through the layer stack.
///
/// Bounds two things at once: how much weight decoding is amortised across
/// tokens, and how long an expired deadline can go unnoticed. A chunk cannot be
/// abandoned partway — the recurrent state would then describe a token the KV
/// cache never saw — so this is the granularity of both.
const PREFILL_CHUNK_TOKENS: usize = 64;

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

/// A rejection of what the *caller* asked for, typed rather than bare.
///
/// Validation here used plain `bail!`, which produces an `anyhow` error
/// indistinguishable from a host fault once it reaches
/// `GenerationError::from_anyhow` — so a request this runtime refused outright
/// reached the client as a retryable 500 and was retried forever. The generic
/// Candle runtime already had a typed variant for exactly this; reusing it
/// keeps one classification rather than two that can drift.
fn invalid_request(alias: &str, detail: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(super::candle_llm_runtime::CandleLlmError::InvalidRequest {
        alias: alias.to_owned(),
        detail: detail.into(),
    })
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
    /// Absent when the caller named no budget, which is what lets the default
    /// be clamped to the context window while an explicit request that cannot
    /// fit is refused. Resolved into `ParsedRequest::max_new_tokens`.
    #[serde(default)]
    max_new_tokens: Option<usize>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    stop: Vec<String>,
    /// Wall-clock budget, in milliseconds. This runtime parses its own request
    /// shape rather than reusing `candle_llm_runtime`'s, so the field has to be
    /// declared here too — omitting it silently ignored the caller's budget and
    /// let a slow CPU generation hold its scheduler lane indefinitely.
    #[serde(default)]
    max_generation_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct IncomingChatTurn {
    role: String,
    content: Value,
}

/// The budget a request that omits `max_new_tokens` gets.
///
/// Shared with the generic Candle runtime rather than held separately. It was
/// 64 here, so once `guest-openai` stopped sending its own default the same
/// bare request answered in 1024 tokens on one local backend and 64 on this
/// one — truncating mid-function on exactly the agentic workloads the raised
/// default exists for. Which runtime happens to serve an alias is not
/// something a caller chooses, so it must not change the answer's length.
fn default_max_new_tokens() -> usize {
    super::candle_llm_runtime::DEFAULT_MAX_NEW_TOKENS
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
        // Which of the two paths runs is settled by what the caller asked for
        // and what candle can reach — not by a probe of the device, and no
        // longer by an environment variable naming one.
        //
        // A `cpu` route is the dense path by definition: unpacking is the only
        // thing a host has to multiply, so it needs no permission. A `gpu`
        // route is a statement that the accelerator will do the work, and a
        // build that cannot make good on it fails to load rather than quietly
        // serving the same tokens eight times slower off the wrong device.
        // That refusal is what `TACHYON_NVFP4_NATIVE_REQUIRED=1` used to buy,
        // except it bought it by asking about hardware; asking for a GPU says
        // the same thing and is the caller's own words.
        let (availability, fallback) = if requested_device == "gpu" {
            let fallback = if dequantized_fallback_opted_in() {
                Nvfp4FallbackPolicy::Permitted
            } else {
                Nvfp4FallbackPolicy::Refused
            };
            (Nvfp4KernelAvailability::detect(), fallback)
        } else {
            (
                Nvfp4KernelAvailability::Absent,
                Nvfp4FallbackPolicy::Permitted,
            )
        };
        let execution_plan = model.select_execution_plan(
            availability,
            Nvfp4OutputDType::F32,
            Nvfp4FallbackScope::LayerWindow(1),
            fallback_limits,
            fallback,
        )?;
        let executed_on = match (&execution_plan, requested_device) {
            (Nvfp4ExecutionPlan::Native, _) => "gpu_native_fp4",
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

    pub(crate) fn generate(
        &self,
        prompts: &[&[u8]],
    ) -> Result<(Vec<u8>, TokenUsage, Option<&'static str>)> {
        let mut output = String::new();
        // A buffered caller is holding the call and cannot go away mid-decode.
        let (usage, finish_reason) = self.generate_streaming(prompts, &mut |delta| {
            output.push_str(delta);
            StreamControl::Continue
        })?;
        Ok((output.into_bytes(), usage, finish_reason))
    }

    pub(crate) fn generate_streaming(
        &self,
        prompts: &[&[u8]],
        on_token: &mut dyn FnMut(&str) -> StreamControl,
    ) -> Result<(TokenUsage, Option<&'static str>)> {
        if prompts.len() != 1 {
            bail!("Qwen 3.5 MoE runtime currently accepts exactly one prompt per decode");
        }
        let request = self.parse_request(prompts[0])?;
        let encoded = self
            .tokenizer
            .encode(request.prompt.clone(), true)
            .map_err(|error| anyhow!("failed to tokenize Qwen prompt: {error}"))?;
        if encoded.is_empty() {
            return Err(invalid_request(
                &self.alias,
                "prompt encoded to zero tokens",
            ));
        }
        // The two cases differ, and conflating them is what made the raised
        // default a regression here: a caller that named a budget has an
        // expectation to violate, so an impossible one is an error, but a
        // caller that named none has none, so the default is clamped to what
        // the window can deliver. Refusing it outright turned prompts that fit
        // comfortably under the old 256-token default into hard failures the
        // moment the shared default became 1024.
        let headroom = self
            .config
            .max_position_embeddings
            .saturating_sub(encoded.len());
        match request.requested_max_new_tokens {
            Some(requested) if requested > headroom => {
                return Err(invalid_request(
                    &self.alias,
                    format!(
                        "prompt tokens {} plus max_new_tokens {requested} exceed the {}-token context limit; lower max_new_tokens to at most {headroom}",
                        encoded.len(),
                        self.config.max_position_embeddings
                    ),
                ));
            }
            _ => {}
        }
        let max_new_tokens = request.max_new_tokens.min(headroom);
        if max_new_tokens == 0 {
            return Err(invalid_request(
                &self.alias,
                format!(
                    "prompt tokens {} leave no room to generate within the {}-token context limit",
                    encoded.len(),
                    self.config.max_position_embeddings
                ),
            ));
        }

        let mut state = HybridDecodeState::new(&self.config.layer_types);
        let mut logits = Vec::new();
        let prompt_ids = encoded.get_ids();
        // Between chunks, not between tokens. A chunk is one pass through the
        // layer stack and cannot be interrupted partway without leaving the
        // recurrent state describing a token the KV cache has not seen, so the
        // chunk size is what bounds how long the deadline check can be delayed
        // — the same trade the generic Candle prefill makes.
        for (chunk_index, chunk) in prompt_ids.chunks(PREFILL_CHUNK_TOKENS).enumerate() {
            let position = chunk_index * PREFILL_CHUNK_TOKENS;
            if position > 0 && Instant::now() >= request.deadline {
                // Not an error: the deadline's contract is that it stops
                // generation and returns what was produced, and a prompt that
                // outlasts prefill produced nothing. Failing here made a long
                // prompt on this CPU-bound runtime a hard failure rather than
                // an empty answer — and the generic runtime made the same
                // mistake, in each of its own prefills.
                return Ok((
                    TokenUsage {
                        prompt_tokens: encoded.len() as u32,
                        completion_tokens: 0,
                    },
                    None,
                ));
            }
            // Only the prompt's last token needs logits; every earlier one
            // exists to advance the state.
            let last = position + chunk.len() == prompt_ids.len();
            logits = self.forward_tokens(chunk, position, &mut state, last)?;
        }
        // The loop above is the only producer, and only its last iteration asks
        // for logits. Stated here because a future edit to either condition
        // would otherwise surface as a tensor built from an empty slice, which
        // says nothing about what actually went wrong.
        if logits.is_empty() {
            bail!("prefill produced no logits for the prompt's final token");
        }
        let sampling = resolve_sampling(
            request.temperature,
            request.top_p,
            request.seed.unwrap_or(299_792_458),
        );
        let mut processor =
            LogitsProcessor::from_sampling(request.seed.unwrap_or(299_792_458), sampling);
        let mut generated = Vec::<u32>::new();
        // Every token this decode produced, EOS included — which is what
        // `completion_tokens` reports and what `generated` deliberately does
        // not hold.
        let mut sampled = 0u32;
        let mut emitted = 0usize;
        let mut abandoned = false;
        // Incremental, like every other production decode loop. Re-decoding the
        // whole sequence after each token is quadratic in the generation
        // length, and the stop check needs the decoded text on every step — so
        // at the shared multi-thousand-token ceiling a long answer spent a
        // large share of its wall-clock budget re-rendering text it had already
        // rendered, while holding the lane.
        let mut decoder = IncrementalDecoder::from_tokenizer(&self.tokenizer);
        // Stop sequences, the held-back tail, and the finish-reason rule, all
        // from upstream — the same `StopCriteria` the generic Candle runtime
        // uses, so the two backends cannot drift apart on them again.
        //
        // The hold is new here. This loop emitted up to `text.len()` on every
        // step, so a stop sequence straddling two tokens had its first half
        // streamed before the match was ever found.
        let criteria = StopCriteria::new(request.stop.clone(), vec![self.config.eos_token_id]);
        let mut finish = None;
        for step in 0..max_new_tokens {
            // Elapsed budget stops generation the way an exhausted token budget
            // does, rather than failing: freeing the scheduler slot is the
            // point, and a partial answer beats an error.
            if Instant::now() >= request.deadline {
                break;
            }
            // Same reasoning for a consumer that went away: finishing an
            // answer nobody will read only occupies the slot.
            if abandoned {
                break;
            }
            let tensor = Tensor::from_vec(logits, self.config.vocab_size, &Device::Cpu)?;
            let token = processor.sample(&tensor)?;
            // Counted where it is produced, not where it is kept. `generated`
            // holds what gets decoded into text, and EOS never joins it —
            // pushing it would emit its literal form. But it *was* generated:
            // it cost a forward pass and a sampling step, and the generic
            // Candle runtime counts it, so reading the count off `generated`
            // made the same request report different `completion_tokens`
            // depending on which backend served the alias.
            sampled += 1;
            if criteria.is_eos(token) {
                finish = criteria.finish_reason(token, decoder.text(), false);
                break;
            }
            generated.push(token);
            decoder
                .push(token)
                .map_err(|error| anyhow!("failed to decode Qwen tokens: {error}"))?;
            let text = decoder.text();
            let safe_end = criteria.safe_emit_end(text);
            if emitted < safe_end {
                abandoned = on_token(&text[emitted..safe_end]).is_stop();
                emitted = safe_end;
            }
            if criteria.matched(text).is_some() {
                finish = criteria.finish_reason(token, text, false);
                break;
            }
            // Decode always needs logits: every step samples from them, and a
            // single token is simply the batch of one.
            logits = self.forward_tokens(&[token], encoded.len() + step, &mut state, true)?;
        }
        // Flush what the hold kept back. Every exit from the loop leaves a
        // suffix unsent — EOS breaks before the token is even pushed — and
        // without this the answer loses its last few bytes.
        let text = decoder.text();
        let end = criteria.matched(text).unwrap_or(text.len());
        if !abandoned && emitted < end {
            on_token(&text[emitted..end]);
        }
        let usage = TokenUsage {
            prompt_tokens: encoded.len() as u32,
            completion_tokens: sampled,
        };
        // Same rule as the generic Candle runtime, and now literally the same
        // code: only budget exhaustion is named, because that is the case an
        // absent reason would misreport as a clean `stop`.
        let finish_reason = match finish {
            Some(FinishReason::Stop) => None,
            Some(FinishReason::Length) => Some("length"),
            None => (generated.len() >= max_new_tokens).then_some("length"),
        };
        Ok((usage, finish_reason))
    }

    fn parse_request(&self, bytes: &[u8]) -> Result<ParsedRequest> {
        // Derived from this checkpoint's context window, the way the generic
        // Candle runtime derives its own. A flat 16 KiB is roughly four
        // thousand tokens of code, so on a model whose window has room for many
        // times that, an agentic client sending a file plus its tool
        // definitions was refused before anything looked at whether it fit.
        // That is the regression `prompt_limits_for` was written to end, and
        // this path was still carrying the constant it replaced.
        let (_max_prompt_tokens, max_prompt_bytes) =
            prompt_limits_for(self.config.max_position_embeddings);
        if bytes.len() > max_prompt_bytes {
            return Err(invalid_request(
                &self.alias,
                format!(
                    "generation request is {} bytes, over the {max_prompt_bytes}-byte limit this \
                     checkpoint's {}-token context window allows",
                    bytes.len(),
                    self.config.max_position_embeddings
                ),
            ));
        }
        let raw = std::str::from_utf8(bytes).context("Qwen prompt must be UTF-8")?;
        let request = if raw.trim_start().starts_with('{') {
            serde_json::from_str::<GenerationRequest>(raw)
                .context("invalid Qwen JSON generation request")?
        } else {
            GenerationRequest {
                prompt: Some(raw.to_owned()),
                messages: None,
                max_new_tokens: None,
                temperature: None,
                top_p: None,
                seed: None,
                stop: Vec::new(),
                max_generation_ms: None,
            }
        };
        if matches!(request.max_new_tokens, Some(requested) if requested == 0 || requested > HOST_MAX_NEW_TOKENS)
        {
            return Err(invalid_request(
                &self.alias,
                format!("max_new_tokens must be between 1 and {HOST_MAX_NEW_TOKENS}"),
            ));
        }
        // Same bounds and same default as the Candle runtime, so a caller sees
        // one wall-clock contract whichever backend serves its alias.
        let budget = match request.max_generation_ms {
            None => DEFAULT_GENERATION_DEADLINE,
            Some(millis) => {
                let requested = Duration::from_millis(millis);
                if requested.is_zero() || requested > HOST_MAX_GENERATION_DEADLINE {
                    return Err(invalid_request(
                        &self.alias,
                        format!(
                            "max_generation_ms {millis} must be between 1 and {}",
                            HOST_MAX_GENERATION_DEADLINE.as_millis()
                        ),
                    ));
                }
                requested
            }
        };
        let deadline = Instant::now() + budget;
        let prompt = match (request.messages, request.prompt) {
            (Some(messages), _) if messages.is_empty() => {
                return Err(invalid_request(
                    &self.alias,
                    "chat request must contain at least one message",
                ))
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
                            // This runtime's request shape carries no tool
                            // history yet, so there is nothing to relay. The
                            // fields exist so the template sees the same
                            // message shape whichever runtime built it.
                            tool_calls: None,
                            tool_call_id: None,
                            name: None,
                            function_call: None,
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
            (None, None) => {
                return Err(invalid_request(
                    &self.alias,
                    "generation request requires `messages` or `prompt`",
                ))
            }
        };
        Ok(ParsedRequest {
            prompt,
            max_new_tokens: request
                .max_new_tokens
                .unwrap_or_else(default_max_new_tokens),
            requested_max_new_tokens: request.max_new_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
            seed: request.seed,
            stop: request
                .stop
                .into_iter()
                .filter(|stop| !stop.is_empty() && stop.len() <= 256)
                .take(8)
                .collect(),
            deadline,
        })
    }

    /// Run `tokens` through the stack, advancing `state` by one position each.
    ///
    /// One function for prefill and decode, with the batch size as the only
    /// difference: decode calls it with a single token, prefill with a chunk of
    /// the prompt. Two functions would have meant two copies of a forty-layer
    /// hybrid loop, and the second copy is where the drift starts.
    ///
    /// Tokens are walked on the *inside* of the layer loop rather than the
    /// outside. That inversion is the point: every projection is then one
    /// `linear_batch` over the whole chunk, so a quantized operator's weights
    /// are unpacked once instead of once per token. What stays per token is
    /// what is genuinely sequential — the linear-attention recurrence, the KV
    /// append, and the mixture-of-experts routing, which picks different
    /// experts for different tokens.
    ///
    /// `want_logits` decides whether the final norm and `lm_head` run at all,
    /// and only ever for the chunk's last token. Prefill overwrites its logits
    /// on every step and samples from the final one, so projecting earlier
    /// tokens through a `vocab_size x hidden_size` operator produced values
    /// that were immediately discarded. The state updates all happen before the
    /// head, which is why skipping it changes nothing the next token sees.
    fn forward_tokens(
        &self,
        tokens: &[u32],
        start_position: usize,
        state: &mut HybridDecodeState,
        want_logits: bool,
    ) -> Result<Vec<f32>> {
        let hidden_size = self.config.hidden_size;
        let count = tokens.len();
        if count == 0 {
            bail!("forward pass requires at least one token");
        }
        let embedding = self
            .model
            .tensor_info("model.language_model.embed_tokens.weight")?;
        let mut hidden = Vec::with_capacity(count * hidden_size);
        for &token in tokens {
            let token = usize::try_from(token)?;
            if token >= self.config.vocab_size {
                bail!("token id {token} exceeds vocabulary");
            }
            hidden.extend_from_slice(&embedding.read_f32_slice(token * hidden_size, hidden_size)?);
        }

        // Normalize every row of `source` with `weight`, returning the same
        // `[count, hidden_size]` layout the projections expect.
        let normalize_rows = |source: &[f32], weight: &[f32]| -> Result<Vec<f32>> {
            let mut out = Vec::with_capacity(count * hidden_size);
            for token in 0..count {
                out.extend(rms_norm_qwen(
                    &source[token * hidden_size..(token + 1) * hidden_size],
                    weight,
                    self.config.rms_norm_eps as f32,
                )?);
            }
            Ok(out)
        };

        for layer in 0..self.config.num_hidden_layers {
            let prefix = format!("model.language_model.layers.{layer}");
            let norm_weight = self
                .model
                .tensor_info(&format!("{prefix}.input_layernorm.weight"))?
                .read_f32()?;
            let normalized = normalize_rows(&hidden, &norm_weight)?;
            let mixed = match (&self.config.layer_types[layer], &mut state.layers[layer]) {
                (LayerType::LinearAttention, LayerDecodeState::Linear(layer_state)) => {
                    let qkv = self.linear_batch(
                        &format!("{prefix}.linear_attn.in_proj_qkv"),
                        &normalized,
                        count,
                    )?;
                    let z = self.linear_batch(
                        &format!("{prefix}.linear_attn.in_proj_z"),
                        &normalized,
                        count,
                    )?;
                    let b = self.linear_batch(
                        &format!("{prefix}.linear_attn.in_proj_b"),
                        &normalized,
                        count,
                    )?;
                    let a = self.linear_batch(
                        &format!("{prefix}.linear_attn.in_proj_a"),
                        &normalized,
                        count,
                    )?;
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
                    let (qkv_width, z_width, b_width, a_width) = (
                        qkv.len() / count,
                        z.len() / count,
                        b.len() / count,
                        a.len() / count,
                    );
                    // Sequential by nature: each step reads the recurrent state
                    // the previous one wrote.
                    let mut cores = Vec::new();
                    for token in 0..count {
                        cores.extend(gated_delta_step(
                            &qkv[token * qkv_width..(token + 1) * qkv_width],
                            &z[token * z_width..(token + 1) * z_width],
                            &b[token * b_width..(token + 1) * b_width],
                            &a[token * a_width..(token + 1) * a_width],
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
                        )?);
                    }
                    self.linear_batch(&format!("{prefix}.linear_attn.out_proj"), &cores, count)?
                }
                (LayerType::FullAttention, LayerDecodeState::Full(layer_state)) => {
                    let query = self.linear_batch(
                        &format!("{prefix}.self_attn.q_proj"),
                        &normalized,
                        count,
                    )?;
                    let key = self.linear_batch(
                        &format!("{prefix}.self_attn.k_proj"),
                        &normalized,
                        count,
                    )?;
                    let value = self.linear_batch(
                        &format!("{prefix}.self_attn.v_proj"),
                        &normalized,
                        count,
                    )?;
                    let q_norm = self
                        .model
                        .tensor_info(&format!("{prefix}.self_attn.q_norm.weight"))?
                        .read_f32()?;
                    let k_norm = self
                        .model
                        .tensor_info(&format!("{prefix}.self_attn.k_norm.weight"))?
                        .read_f32()?;
                    let (query_width, key_width, value_width) =
                        (query.len() / count, key.len() / count, value.len() / count);
                    // Also sequential: each token appends to the KV cache the
                    // next one attends over.
                    let mut cores = Vec::new();
                    for token in 0..count {
                        cores.extend(full_attention_step(
                            &query[token * query_width..(token + 1) * query_width],
                            &key[token * key_width..(token + 1) * key_width],
                            &value[token * value_width..(token + 1) * value_width],
                            &q_norm,
                            &k_norm,
                            self.config.num_attention_heads,
                            self.config.num_key_value_heads,
                            self.config.head_dim,
                            (self.config.head_dim as f64 * self.config.partial_rotary_factor)
                                as usize,
                            self.config.rope_parameters.rope_theta as f32,
                            start_position + token,
                            layer_state,
                        )?);
                    }
                    self.linear_batch(&format!("{prefix}.self_attn.o_proj"), &cores, count)?
                }
                _ => bail!("hybrid decode state does not match layer {layer}"),
            };
            for token in 0..count {
                let (row, mixed_row) = (
                    &mut hidden[token * hidden_size..(token + 1) * hidden_size],
                    &mixed[token * hidden_size..(token + 1) * hidden_size],
                );
                add_assign(row, mixed_row)?;
            }

            let post_norm_weight = self
                .model
                .tensor_info(&format!("{prefix}.post_attention_layernorm.weight"))?
                .read_f32()?;
            let post_norm = normalize_rows(&hidden, &post_norm_weight)?;
            let router = self.linear_batch(&format!("{prefix}.mlp.gate"), &post_norm, count)?;
            let shared_gate = self.linear_batch(
                &format!("{prefix}.mlp.shared_expert_gate"),
                &post_norm,
                count,
            )?;
            let moe = sparse_moe_forward_batch(
                &post_norm,
                &router,
                count,
                self.config.num_experts_per_tok,
                |expert, inputs, tokens| {
                    self.mlp(&format!("{prefix}.mlp.experts.{expert}"), inputs, tokens)
                },
                |inputs, tokens| self.mlp(&format!("{prefix}.mlp.shared_expert"), inputs, tokens),
                &shared_gate,
            )?;
            for token in 0..count {
                let (row, moe_row) = (
                    &mut hidden[token * hidden_size..(token + 1) * hidden_size],
                    &moe[token * hidden_size..(token + 1) * hidden_size],
                );
                add_assign(row, moe_row)?;
            }
        }

        if !want_logits {
            return Ok(Vec::new());
        }
        let final_norm = self
            .model
            .tensor_info("model.language_model.norm.weight")?
            .read_f32()?;
        let last = &hidden[(count - 1) * hidden_size..];
        let hidden = rms_norm_qwen(last, &final_norm, self.config.rms_norm_eps as f32)?;
        self.linear("lm_head", &hidden)
    }

    /// The feed-forward block for `tokens` activations at once.
    ///
    /// SwiGLU is elementwise, so batching it is only a matter of running the
    /// three projections over the whole `[tokens, hidden_size]` block — which
    /// is where the saving is, since each of them otherwise decodes its
    /// quantized weights once per token.
    fn mlp(&self, prefix: &str, inputs: &[f32], tokens: usize) -> Result<Vec<f32>> {
        let gate = self.linear_batch(&format!("{prefix}.gate_proj"), inputs, tokens)?;
        let up = self.linear_batch(&format!("{prefix}.up_proj"), inputs, tokens)?;
        let activated = gate
            .into_iter()
            .zip(up)
            .map(|(gate, up)| primitives::silu(gate) * up)
            .collect::<Vec<_>>();
        self.linear_batch(&format!("{prefix}.down_proj"), &activated, tokens)
    }

    /// Apply `base` to `tokens` activations at once.
    ///
    /// `inputs` is `[tokens, cols]` row-major and the result is
    /// `[tokens, rows]`. One call decodes the operator's weights once instead
    /// of once per token, which is the whole reason the layer loop below walks
    /// tokens on the inside rather than the outside.
    fn linear_batch(&self, base: &str, inputs: &[f32], tokens: usize) -> Result<Vec<f32>> {
        let prepared = self.prepared_linear(base)?;
        match &self.execution_plan {
            Nvfp4ExecutionPlan::Native => prepared.matmul_native_nvfp4(inputs, tokens),
            Nvfp4ExecutionPlan::Fallback(_) => prepared.matmul(inputs, tokens),
        }
        .with_context(|| format!("Qwen operator `{base}` failed"))
    }

    /// The working-set lookup both `linear` and `linear_batch` go through.
    fn prepared_linear(&self, base: &str) -> Result<Arc<PreparedLinear>> {
        let cached = self
            .working_set
            .lock()
            .map_err(|_| anyhow!("Qwen working-set lock is poisoned"))?
            .get(base);
        match cached {
            Some(prepared) => Ok(prepared),
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
                Ok(prepared)
            }
        }
    }

    fn linear(&self, base: &str, input: &[f32]) -> Result<Vec<f32>> {
        let prepared = self.prepared_linear(base)?;
        match &self.execution_plan {
            Nvfp4ExecutionPlan::Native => prepared.matvec_native_nvfp4(input),
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
    /// What the caller actually asked for, or `None` when it named no budget.
    /// Only an explicit request is refused when it cannot fit the window; an
    /// unstated one is clamped, exactly as the generic Candle runtime does.
    requested_max_new_tokens: Option<usize>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    seed: Option<u64>,
    stop: Vec<String>,
    /// Absolute, anchored at parse time so tokenizing and prefilling count
    /// against the budget — that work holds the same scheduler slot.
    deadline: Instant,
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

    /// A GPU route runs the kernel or it does not load.
    ///
    /// This is the property `TACHYON_NVFP4_NATIVE_REQUIRED=1` was buying, and
    /// it was buying it in the wrong currency: an environment variable about
    /// hardware, set by whoever remembered to. It belongs to the loader.
    /// Asking for an accelerator is a claim about where the work happens, so
    /// either the packed path runs or the load fails — never a silent descent
    /// to the host path, which would answer with the same tokens and none of
    /// the meaning.
    ///
    /// Both halves are asserted because both are reachable. `candle-cuda` is
    /// what compiles CUDA support into the kernel crate, so with it the packed
    /// path must be reached, and without it the load must refuse rather than
    /// fall back. Skipped only where candle reports no CUDA device — that is
    /// the hardware speaking, and it is the sole excuse this test accepts.
    #[test]
    fn gated_a_gpu_route_runs_the_kernel_or_refuses_to_load() {
        let Ok(path) = std::env::var("TACHYON_QWEN35_MOE_NVFP4_DIR") else {
            return;
        };
        if candle_core::Device::new_cuda(0).is_err() {
            eprintln!("skipping GPU-route selection: no CUDA device");
            return;
        }

        let loaded = Qwen35MoeRuntime::try_load("qwen35-gpu-route", &path, "gpu");

        if cfg!(feature = "candle-cuda") {
            let runtime = loaded
                .expect("a GPU route must load where the kernel is reachable")
                .expect("checkpoint should select the Qwen runtime");
            assert_eq!(runtime.executed_on(), "gpu_native_fp4");
        } else {
            let Err(error) = loaded else {
                panic!("a build with no kernel must refuse a GPU route, not load one");
            };
            assert!(
                error.to_string().contains("no usable NVFP4 kernel"),
                "the refusal should name the missing kernel, got: {error}"
            );
        }
    }

    /// Chunk size must not change the answer.
    ///
    /// This is the whole safety argument for walking tokens inside the layer
    /// loop. Batching moves *where* the projections are computed, never what
    /// they compute: `PreparedLinear::matmul` keeps each output element's
    /// accumulation order, and the recurrence, the KV append and the expert
    /// routing all still run one token at a time. If that holds, prefilling a
    /// prompt in one pass and prefilling it one token at a time must produce
    /// the same logits — bit for bit, not approximately.
    ///
    /// Gated on a real checkpoint because it is the only place the hybrid
    /// stack can actually run: there is no synthetic Qwen 3.5 fixture, so this
    /// executes on the GPU runner and nowhere else. That is a real gap in the
    /// safety net and the reason this test compares exactly rather than within
    /// a tolerance — a loose comparison here would leave the restructure
    /// effectively unverified.
    #[test]
    fn gated_prefill_is_invariant_to_chunk_size() {
        let Ok(path) = std::env::var("TACHYON_QWEN35_MOE_NVFP4_DIR") else {
            return;
        };
        let runtime = Qwen35MoeRuntime::try_load("qwen35-chunking", &path, "cpu")
            .expect("runtime construction should succeed")
            .expect("checkpoint should select the Qwen runtime");

        // Long enough to span several chunks, so the boundary logic is
        // exercised rather than skipped.
        let prompt = "the mesh routes a prompt through many layers ".repeat(8);
        let encoded = runtime
            .tokenizer
            .encode(prompt, true)
            .expect("prompt should tokenize");
        let ids = encoded.get_ids();
        assert!(
            ids.len() > PREFILL_CHUNK_TOKENS,
            "the prompt must span more than one chunk to test anything"
        );

        let run = |chunk_size: usize| -> Vec<f32> {
            let mut state = HybridDecodeState::new(&runtime.config.layer_types);
            let mut logits = Vec::new();
            for (index, chunk) in ids.chunks(chunk_size).enumerate() {
                let position = index * chunk_size;
                let last = position + chunk.len() == ids.len();
                logits = runtime
                    .forward_tokens(chunk, position, &mut state, last)
                    .expect("prefill should succeed");
            }
            logits
        };

        let one_at_a_time = run(1);
        let batched = run(PREFILL_CHUNK_TOKENS);
        assert_eq!(
            one_at_a_time.len(),
            batched.len(),
            "both paths produce logits for the prompt's last token"
        );
        assert_eq!(
            one_at_a_time, batched,
            "batching changed the result, so it changed more than where the \
             projections are computed"
        );
    }

    /// The proof that would let the scalar runtime be deleted.
    ///
    /// Two implementations of one architecture is a liability, and the only
    /// honest way out is to show they answer the same question on a real
    /// checkpoint. Exact equality is not available and asking for it would be a
    /// mistake: the scalar path accumulates each output element in a fixed
    /// order, upstream's goes through a GEMM whose blocking is its own
    /// business. What must agree is the answer — the token the model would
    /// emit — and the logits either side of it.
    ///
    /// Gated on an installed checkpoint and a CUDA device — no longer on FP4
    /// tensor cores, which candle #3831 established the kernel never needed.
    /// What remains is `TACHYON_ENABLE_NVFP4_CI` and the weights it implies:
    /// without them this test compiles and does nothing, which is worth stating
    /// plainly rather than discovering later.
    #[test]
    fn gated_upstream_layers_agree_with_the_scalar_runtime() {
        let Ok(path) = std::env::var("TACHYON_QWEN35_MOE_NVFP4_DIR") else {
            return;
        };
        let device = match candle_core::Device::new_cuda(0) {
            Ok(device) => device,
            Err(_) => {
                eprintln!("skipping upstream-layer parity: no CUDA device");
                return;
            }
        };
        let runtime = Qwen35MoeRuntime::try_load("qwen35-parity", &path, "cpu")
            .expect("runtime construction should succeed")
            .expect("checkpoint should select the Qwen runtime");
        let model = ModelOptNvfp4Directory::try_load("qwen35-parity", &path)
            .expect("checkpoint parser should succeed")
            .expect("checkpoint should be detected as ModelOpt/NVFP4");
        let mut upstream =
            match crate::ai_inference::qwen35_upstream::load(&model, &runtime.config, &device) {
                Ok(model) => model,
                Err(error) => {
                    eprintln!("skipping upstream-layer parity: {error:#}");
                    return;
                }
            };

        let encoded = runtime
            .tokenizer
            .encode("the mesh routes a prompt through many layers", true)
            .expect("prompt should tokenize");
        let ids = encoded.get_ids();

        let mut state = HybridDecodeState::new(&runtime.config.layer_types);
        let scalar = runtime
            .forward_tokens(ids, 0, &mut state, true)
            .expect("scalar prefill should succeed");

        let input = candle_core::Tensor::new(ids, &device)
            .and_then(|ids| ids.unsqueeze(0))
            .expect("prompt should become a tensor");
        let logits = upstream
            .forward(&input, 0)
            .and_then(|logits| logits.flatten_all()?.to_vec1::<f32>())
            .expect("upstream prefill should succeed");

        assert_eq!(
            scalar.len(),
            logits.len(),
            "both paths produce one logit per vocabulary entry"
        );
        let argmax = |values: &[f32]| {
            values
                .iter()
                .enumerate()
                .fold((0usize, f32::NEG_INFINITY), |best, (index, &value)| {
                    if value > best.1 {
                        (index, value)
                    } else {
                        best
                    }
                })
                .0
        };
        assert_eq!(
            argmax(&scalar),
            argmax(&logits),
            "the two implementations would emit different tokens"
        );

        // Beyond the argmax: the distributions themselves have to line up, or
        // the two paths agree here and diverge on the next sample. Scaled by
        // the logit range so the bound means the same thing on any checkpoint.
        let range = scalar.iter().copied().fold(f32::NEG_INFINITY, f32::max)
            - scalar.iter().copied().fold(f32::INFINITY, f32::min);
        let worst = scalar
            .iter()
            .zip(&logits)
            .map(|(left, right)| (left - right).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst < range * 0.01,
            "largest logit difference {worst} exceeds 1% of the {range} logit range"
        );
    }

    /// Measures how prefill scales with prompt length, and reports it.
    ///
    /// Gated twice: on a real NVFP4 checkpoint, and on an explicit opt-in,
    /// because it is a wall-clock measurement and belongs on a machine chosen
    /// for it rather than in every developer's `cargo test`.
    ///
    /// The assertion is deliberately loose. Absolute throughput is a property
    /// of the host, not of this code, so pinning a number here would fail on
    /// every machine that is not the one it was written on. What *is* a
    /// property of this code is the shape of the curve: prefill runs one
    /// matrix-*vector* product per token per projection, so doubling the prompt
    /// should roughly double the time. Anything worse than quadratic-ish growth
    /// means a per-token cost that itself grows with position — a cache that
    /// stopped hitting, a state that is being copied rather than updated — and
    /// that is a defect at any absolute speed.
    ///
    /// The numbers it prints are the baseline for the batched-projection work:
    /// a `matmul` over the whole prompt reads each weight once instead of once
    /// per token, and this is what will show it.
    #[test]
    fn gated_prefill_scaling_is_reported_and_not_superlinear() {
        let Ok(path) = std::env::var("TACHYON_QWEN35_MOE_NVFP4_DIR") else {
            return;
        };
        if std::env::var("TACHYON_QWEN35_PREFILL_BENCH").as_deref() != Ok("1") {
            return;
        }
        let runtime = Qwen35MoeRuntime::try_load("qwen35-bench", &path, "cpu")
            .expect("runtime construction should succeed")
            .expect("checkpoint should select the Qwen runtime");

        // One decoded token, so the measurement is prefill plus a fixed
        // constant rather than prefill plus a generation whose length varies.
        let measure = |words: usize| -> std::time::Duration {
            let prompt = "token ".repeat(words);
            let request = serde_json::json!({
                "prompt": prompt,
                "max_new_tokens": 1,
                "temperature": 0.0,
            })
            .to_string();
            let started = Instant::now();
            runtime
                .generate(&[request.as_bytes()])
                .expect("prefill benchmark generation should succeed");
            started.elapsed()
        };

        // Warm the working set first: the first call pays for preparing every
        // projection, which is a one-off and not what is being measured.
        let _ = measure(16);

        let short = measure(64);
        let long = measure(128);
        let ratio = long.as_secs_f64() / short.as_secs_f64().max(f64::EPSILON);
        println!(
            "qwen35 prefill: 64 tokens {short:?}, 128 tokens {long:?}, ratio {ratio:.2}, \
             {:.1} tok/s at 128",
            128.0 / long.as_secs_f64()
        );
        assert!(
            ratio < 4.0,
            "doubling the prompt should not cost more than ~4x; got {ratio:.2} \
             ({short:?} -> {long:?}), which means per-token cost grows with position"
        );
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

    /// This runtime parses its own request shape instead of reusing
    /// `candle_llm_runtime`'s, so a field missing from *this* struct is
    /// silently dropped by serde rather than rejected. That is how
    /// `max_generation_ms` came to be accepted by the API, documented as a
    /// wall-clock budget, and ignored by this backend entirely.
    #[test]
    fn the_request_shape_carries_the_wall_clock_budget() {
        let request: GenerationRequest =
            serde_json::from_str(r#"{"prompt":"hi","max_new_tokens":4,"max_generation_ms":1500}"#)
                .expect("request should parse");
        assert_eq!(request.max_generation_ms, Some(1500));

        // Omitting it is still valid; the default applies at parse time.
        let defaulted: GenerationRequest =
            serde_json::from_str(r#"{"prompt":"hi"}"#).expect("request should parse");
        assert_eq!(defaulted.max_generation_ms, None);
    }

    /// One wall-clock contract across backends: a caller must not have to know
    /// which runtime serves its alias to know what budget it gets.
    /// This backend held nothing back: it streamed up to `text.len()` on every
    /// step, so a stop sequence straddling two tokens had its first half sent
    /// before the match was found — the client saw bytes the buffered response
    /// never contains. Sharing `StopCriteria` with the generic runtime is what
    /// fixed it, and this pins the property rather than the implementation.
    #[test]
    fn a_stop_sequence_split_across_tokens_is_never_partially_emitted() {
        let criteria = StopCriteria::new(vec!["<|end|>".to_owned()], vec![7]);

        // The withheld tail is a fixed byte count — one short of the longest
        // stop sequence — not a suffix the criteria tries to match. That is
        // deliberately conservative: any prefix of a stop sequence is
        // necessarily inside those bytes, so none of it can be emitted.
        assert_eq!(criteria.hold(), "<|end|>".len() - 1);
        assert_eq!(criteria.safe_emit_end("abc<|end"), "abc<|end".len() - 6);

        // Once it completes, emission stops exactly at the match — the
        // sequence itself is never part of the answer.
        assert_eq!(criteria.safe_emit_end("abc<|end|>"), 3);
        assert_eq!(criteria.matched("abc<|end|>"), Some(3));

        // And with no stop configured nothing is withheld, so an ordinary
        // generation still streams as it is produced.
        let none = StopCriteria::new(Vec::<String>::new(), vec![7]);
        assert_eq!(none.hold(), 0);
        assert_eq!(none.safe_emit_end("abc"), 3);
    }

    /// The boundary the token count cannot express, asserted on the shared
    /// verdict rather than on each backend's copy of the rule.
    #[test]
    fn an_eos_on_the_budgets_last_token_is_a_stop_not_a_length() {
        let criteria = StopCriteria::new(Vec::<String>::new(), vec![7]);
        assert_eq!(
            criteria.finish_reason(7, "done", true),
            Some(FinishReason::Stop),
            "EOS landing on the last allowed token still ended the answer normally"
        );
        assert_eq!(
            criteria.finish_reason(3, "done", true),
            Some(FinishReason::Length),
            "without a model-controlled ending, an exhausted budget is a truncation"
        );
        assert_eq!(
            criteria.finish_reason(3, "done", false),
            None,
            "and a loop that stopped for some other reason names none"
        );
    }

    /// The byte cap follows the checkpoint, not a constant.
    ///
    /// A flat 16 KiB is roughly four thousand tokens of code. On a Qwen 3.5
    /// window there is room for many times that, so an agentic client sending
    /// a file plus its tool definitions was refused before anything looked at
    /// whether the prompt fit. `prompt_limits_for` exists to end exactly that,
    /// and this path was still carrying the constant it replaced.
    #[test]
    fn the_qwen_byte_cap_scales_with_the_context_window() {
        let (_tokens, small) = prompt_limits_for(2_048);
        let (_tokens, large) = prompt_limits_for(262_144);
        assert!(
            large > small,
            "a larger window has to buy a larger prompt, got {large} and {small}"
        );
        assert!(
            large > 16_384,
            "the old flat cap must not still be the ceiling for a large window, got {large}"
        );
        // And a tiny checkpoint keeps a workable floor rather than inheriting a
        // cap proportional to almost nothing.
        assert!(small > 0);
    }

    #[test]
    fn the_deadline_bounds_match_the_candle_runtime() {
        assert_eq!(DEFAULT_GENERATION_DEADLINE, Duration::from_secs(300));
        assert_eq!(HOST_MAX_GENERATION_DEADLINE, Duration::from_secs(3_600));
        assert!(HOST_MAX_GENERATION_DEADLINE > DEFAULT_GENERATION_DEADLINE);
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
        let (buffered, buffered_usage, buffered_finish) =
            runtime.generate(&[request]).expect("buffered generation");
        let first_token_ms = started.elapsed().as_millis();
        let mut streamed = String::new();
        let decode_started = std::time::Instant::now();
        let (streamed_usage, streamed_finish) = runtime
            .generate_streaming(&[request], &mut |delta| {
                streamed.push_str(delta);
                StreamControl::Continue
            })
            .expect("streaming generation");
        let decode_ms = decode_started.elapsed().as_millis();

        assert!(!buffered.is_empty());
        assert_ne!(buffered.as_slice(), b"MOCK_LLM_RESPONSE");
        assert_eq!(String::from_utf8(buffered).expect("UTF-8"), streamed);
        // The buffered wrapper is an accumulator over the streaming core, so
        // the two must agree on what the generation cost as well as on its text.
        assert_eq!(buffered_usage, streamed_usage);
        assert_eq!(buffered_finish, streamed_finish);
        assert!(buffered_usage.completion_tokens > 0);
        // The request asks for one new token, and a completion that ends
        // naturally samples EOS after it. Reading the count off `generated` —
        // which never holds EOS, because pushing it would emit its literal
        // form — reported one token where the generic Candle runtime reports
        // two for the same request, so the figure a client is billed on
        // depended on which backend served the alias.
        assert!(
            buffered_usage.completion_tokens >= 2,
            "a naturally-ended completion counts the EOS it sampled, got {}",
            buffered_usage.completion_tokens
        );
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
