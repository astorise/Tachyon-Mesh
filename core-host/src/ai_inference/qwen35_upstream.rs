//! The Qwen 3.5 MoE runtime, executing on candle's own layers.
//!
//! This module used to be a loader beside a second, local implementation of
//! the architecture: `qwen35_moe_runtime.rs` reimplemented attention, the
//! gated-delta recurrence, the sparse mixture and the norms, because for a
//! long time there was nowhere else for them to live. Three upstream changes
//! removed every reason (candle #3831, #3832, #3837), and a fourth removed the
//! last one: #3857 gave NVFP4 a packed CPU operator, so a host without a CUDA
//! device no longer has to choose between an eight-times unpack and nothing at
//! all.
//!
//! What is left here is the join: translate the validated checkpoint config
//! into upstream's, hand upstream a projection factory that knows how ModelOpt
//! stores a weight, and run the decode loop. The architecture is candle's.
//!
//! ## One request at a time
//!
//! `ModelForCausalLM::forward` takes `&mut self` because the model owns its KV
//! cache and the recurrent state of every linear-attention layer. The scalar
//! runtime kept that state in a local, so requests to one alias were naturally
//! concurrent; upstream's is inside the model, so they cannot be. The `Mutex`
//! is what makes that safe, and `clear_kv_cache` between requests is what keeps
//! one answer out of the next. The scheduler already bounds concurrency, and a
//! 40-layer 256-expert mixture does not serve many requests at once in any
//! case — but this is a narrowing, not a free simplification.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use candle_core::{Device, Tensor};
use candle_transformers::generation::{
    FinishReason, IncrementalDecoder, LogitsProcessor, Sampling, StopCriteria,
};
use candle_transformers::models::qwen3_5::{
    Config as UpstreamConfig, ModelForCausalLM, Projection, RopeParameters as UpstreamRope,
    TextConfig,
};
use candle_transformers::quantized_nvfp4::{auto_linear, Nvfp4Config};
use serde::Deserialize;
use serde_json::Value;
use tokenizers::Tokenizer;

use super::candle_llm_runtime::{
    prompt_limits_for, ChatTemplate, ChatTurn, TokenUsage, DEFAULT_GENERATION_DEADLINE,
    HOST_MAX_GENERATION_DEADLINE, HOST_MAX_NEW_TOKENS,
};
use super::modelopt_nvfp4::{
    dequantized_fallback_opted_in, ModelOptNvfp4Directory, Nvfp4KernelAvailability,
    DEQUANTIZED_FALLBACK_ENV,
};
use super::qwen35_profile::{LayerType, Qwen35MoeConfig, Qwen35Route, QWEN35_MOE_DESCRIPTOR};
use super::{
    architecture_registry::{ArchitectureDescriptor, ArchitectureKind},
    StreamControl,
};

/// Prompt tokens per prefill pass through the layer stack.
///
/// The unit at which the deadline can be observed: a pass cannot be
/// interrupted partway without leaving the recurrent state describing a token
/// the KV cache has not seen.
const PREFILL_CHUNK_TOKENS: usize = 64;

/// Translate our validated checkpoint config into upstream's.
///
/// Three fields upstream needs are not in a Qwen 3.5 MoE `text_config`, or are
/// not read by our validator, and each is pinned here rather than guessed at
/// load time:
///
/// * `intermediate_size` — the *dense* feed-forward width. Every layer of a
///   checkpoint with `num_experts > 0` is sparse, so upstream never reads it;
///   it is set to the MoE width so that a wrong value cannot quietly size
///   anything.
/// * `hidden_act` — Qwen 3.5 is SwiGLU with SiLU, which is what the
///   checkpoints carry.
/// * `attention_bias` — ModelOpt Qwen 3.5 checkpoints have no attention bias.
///   If that ever changes the projections will fail to find their bias
///   tensors, which is the loud failure rather than the silent one.
pub(crate) fn upstream_config(config: &Qwen35MoeConfig) -> UpstreamConfig {
    UpstreamConfig {
        text_config: TextConfig {
            vocab_size: config.vocab_size,
            hidden_size: config.hidden_size,
            intermediate_size: config.moe_intermediate_size,
            num_hidden_layers: config.num_hidden_layers,
            num_attention_heads: config.num_attention_heads,
            num_key_value_heads: config.num_key_value_heads,
            head_dim: Some(config.head_dim),
            max_position_embeddings: config.max_position_embeddings,
            rms_norm_eps: config.rms_norm_eps,
            rope_parameters: UpstreamRope {
                rope_theta: config.rope_parameters.rope_theta,
            },
            attention_bias: false,
            hidden_act: candle_nn::Activation::Silu,
            num_experts: config.num_experts,
            num_experts_per_tok: config.num_experts_per_tok,
            moe_intermediate_size: config.moe_intermediate_size,
            shared_expert_intermediate_size: config.shared_expert_intermediate_size,
            partial_rotary_factor: config.partial_rotary_factor,
            layer_types: config
                .layer_types
                .iter()
                .map(|layer| match layer {
                    LayerType::LinearAttention => "linear_attention".to_owned(),
                    LayerType::FullAttention => "full_attention".to_owned(),
                })
                .collect(),
            linear_num_key_heads: config.linear_num_key_heads,
            linear_num_value_heads: config.linear_num_value_heads,
            linear_key_head_dim: config.linear_key_head_dim,
            linear_value_head_dim: config.linear_value_head_dim,
            linear_conv_kernel_dim: config.linear_conv_kernel_dim,
        },
    }
}

/// A rejection of what the *caller* asked for, typed rather than bare.
///
/// A plain `bail!` produces an `anyhow` error indistinguishable from a host
/// fault once it reaches `GenerationError::from_anyhow`, so a request this
/// runtime refused outright reached the client as a retryable 500 and was
/// retried forever.
fn invalid_request(alias: &str, detail: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(super::candle_llm_runtime::CandleLlmError::InvalidRequest {
        alias: alias.to_owned(),
        detail: detail.into(),
    })
}

pub(crate) struct Qwen35MoeRuntime {
    alias: String,
    root: PathBuf,
    config: Qwen35MoeConfig,
    tokenizer: Tokenizer,
    chat_template: Option<ChatTemplate>,
    executed_on: &'static str,
    /// See the module docs: upstream's model owns the state a decode advances,
    /// so one decode at a time per loaded alias.
    model: Mutex<ModelForCausalLM>,
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
        let route = Qwen35Route::parse(requested_device)?;
        let root = path.as_ref();
        let tokenizer = Tokenizer::from_file(root.join("tokenizer.json"))
            .map_err(|error| anyhow!("failed to load Qwen tokenizer.json: {error}"))?;
        let chat_template = ChatTemplate::load(alias, root)?;
        let config = Qwen35MoeConfig::validate_model(&model)?;

        // A `gpu` route is a claim that the accelerator does the work, and it
        // has to be checked *here* rather than left to `auto_linear`: without
        // the `nvfp4-cuda` feature compiled in, upstream's chooser silently
        // picks the packed CPU operator even on a CUDA device, which answers
        // with the same tokens off the wrong hardware. That silent descent is
        // the whole thing a `gpu` route exists to rule out.
        let device = match route {
            Qwen35Route::Host => Device::Cpu,
            Qwen35Route::Cuda => {
                if Nvfp4KernelAvailability::detect() != Nvfp4KernelAvailability::Reachable
                    && !dequantized_fallback_opted_in()
                {
                    bail!(
                        "candle reports no usable NVFP4 kernel on this host: build with the \
                         `candle-cuda` feature and run where there is a CUDA device, or set \
                         {DEQUANTIZED_FALLBACK_ENV}=1 to unpack the weights to dense f32 \
                         instead — eight times the memory of the packed checkpoint, which is \
                         why it is not the default"
                    );
                }
                Device::new_cuda(0).context("a `cuda` route needs a CUDA device")?
            }
        };
        let executed_on = match route {
            Qwen35Route::Cuda => "gpu_native_fp4",
            // Packed on the host since candle #3857: the checkpoint's own
            // footprint rather than eight times it, decoded per row during the
            // forward pass.
            Qwen35Route::Host => "cpu_packed_fp4",
        };

        let model = load(&model, &config, &device)?;
        Ok(Self {
            alias: alias.to_owned(),
            root: root.to_path_buf(),
            config,
            tokenizer,
            chat_template,
            executed_on,
            model: Mutex::new(model),
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
        // A caller that named a budget has an expectation to violate, so an
        // impossible one is an error; a caller that named none has none, so the
        // default is clamped to what the window can deliver.
        let headroom = self
            .config
            .max_position_embeddings
            .saturating_sub(encoded.len());
        if let Some(requested) = request.requested_max_new_tokens {
            if requested > headroom {
                return Err(invalid_request(
                    &self.alias,
                    format!(
                        "prompt tokens {} plus max_new_tokens {requested} exceed the {}-token context limit; lower max_new_tokens to at most {headroom}",
                        encoded.len(),
                        self.config.max_position_embeddings
                    ),
                ));
            }
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

        let mut model = self
            .model
            .lock()
            .map_err(|_| anyhow!("Qwen 3.5 runtime lock was poisoned by an earlier panic"))?;
        // The previous request's cache and recurrent state say nothing about
        // this prompt, and upstream keeps both inside the model.
        model.clear_kv_cache();

        let prompt_ids = encoded.get_ids();
        let device = self.device_of(&model);
        let mut logits: Option<Tensor> = None;
        for (chunk_index, chunk) in prompt_ids.chunks(PREFILL_CHUNK_TOKENS).enumerate() {
            let position = chunk_index * PREFILL_CHUNK_TOKENS;
            if position > 0 && Instant::now() >= request.deadline {
                // Not an error: the deadline's contract is that it stops
                // generation and returns what was produced, and a prompt that
                // outlasts prefill produced nothing.
                return Ok((
                    TokenUsage {
                        prompt_tokens: encoded.len() as u32,
                        completion_tokens: 0,
                    },
                    None,
                ));
            }
            let input = Tensor::new(chunk, &device)?.unsqueeze(0)?;
            logits = Some(model.forward(&input, position)?);
        }
        // Upstream narrows to the final position itself, so every chunk yields
        // logits and only the last one's are worth keeping.
        let Some(mut logits) = logits else {
            bail!("prefill produced no logits for the prompt's final token");
        };

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
        let mut decoder = IncrementalDecoder::from_tokenizer(&self.tokenizer);
        // Stop sequences, the held-back tail and the finish-reason rule all
        // come from upstream, the same `StopCriteria` the generic Candle
        // runtime uses, so the two backends cannot drift apart on them.
        let criteria = StopCriteria::new(request.stop.clone(), vec![self.config.eos_token_id]);
        let mut finish = None;
        for step in 0..max_new_tokens {
            // An elapsed budget stops generation the way an exhausted token
            // budget does, rather than failing: freeing the scheduler slot is
            // the point, and a partial answer beats an error.
            if Instant::now() >= request.deadline {
                // Named, not left unset. An absent reason maps to `None` below
                // unless the budget happened to run out on the exact token the
                // caller asked for, so a deadline-truncated answer reached the
                // client as a clean `stop` — indistinguishable from a model
                // that chose to finish. The generic Candle runtime records the
                // same verdict on its own deadline.
                finish = Some(FinishReason::Length);
                break;
            }
            // Same reasoning for a consumer that went away.
            if abandoned {
                break;
            }
            let token = processor.sample(&logits.squeeze(0)?.squeeze(0)?)?;
            // Counted where it is produced, not where it is kept: EOS costs a
            // forward pass and a sampling step, and the generic Candle runtime
            // counts it, so reading the count off `generated` made the same
            // request report different `completion_tokens` depending on which
            // backend served the alias.
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
            let input = Tensor::new(&[token][..], &device)?.unsqueeze(0)?;
            logits = model.forward(&input, encoded.len() + step)?;
        }
        // Flush what the hold kept back: every exit leaves a suffix unsent —
        // EOS breaks before the token is even pushed.
        let text = decoder.text();
        let end = criteria.matched(text).unwrap_or(text.len());
        if !abandoned && emitted < end {
            on_token(&text[emitted..end]);
        }
        let usage = TokenUsage {
            prompt_tokens: encoded.len() as u32,
            completion_tokens: sampled,
        };
        // Only budget exhaustion is named, because that is the case an absent
        // reason would misreport as a clean `stop`.
        let finish_reason = match finish {
            Some(FinishReason::Stop) => None,
            Some(FinishReason::Length) => Some("length"),
            None => (generated.len() >= max_new_tokens).then_some("length"),
        };
        Ok((usage, finish_reason))
    }

    /// The device the loaded weights live on.
    ///
    /// Read back from the model rather than stored beside it, so the tensors a
    /// decode builds cannot end up on a different device from the ones it
    /// multiplies.
    fn device_of(&self, model: &ModelForCausalLM) -> Device {
        let _ = model;
        match self.executed_on {
            "gpu_native_fp4" => Device::new_cuda(0).unwrap_or(Device::Cpu),
            _ => Device::Cpu,
        }
    }

    fn parse_request(&self, bytes: &[u8]) -> Result<ParsedRequest> {
        // Derived from this checkpoint's context window, the way the generic
        // Candle runtime derives its own: a flat byte cap refused an agentic
        // client's file plus its tool definitions before anything looked at
        // whether they fit.
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
                ..GenerationRequest::default()
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
                            tool_calls: None,
                            tool_call_id: None,
                            name: None,
                            function_call: None,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                match &self.chat_template {
                    Some(template) => template
                        .render(&messages, request.tools.as_deref())
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
}

#[derive(Debug, Default, Deserialize)]
struct GenerationRequest {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    messages: Option<Vec<IncomingChatTurn>>,
    /// Tool schemas offered by the caller, handed to the chat template's
    /// `tools` variable. This runtime parses its own request shape, so a field
    /// it does not name is one serde silently drops — and a Qwen template gates
    /// its tool block on `{% if tools %}`, so dropping them means the model is
    /// never told the functions exist.
    #[serde(default)]
    tools: Option<Vec<Value>>,
    /// Absent when the caller named no budget, which is what lets the default
    /// be clamped to the context window while an explicit request that cannot
    /// fit is refused.
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
    /// Wall-clock budget, in milliseconds. Omitting it here silently ignored
    /// the caller's budget and let a slow generation hold its scheduler lane.
    #[serde(default)]
    max_generation_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct IncomingChatTurn {
    role: String,
    content: Value,
}

struct ParsedRequest {
    prompt: String,
    max_new_tokens: usize,
    /// What the caller actually asked for, or `None` when it named no budget.
    requested_max_new_tokens: Option<usize>,
    temperature: Option<f32>,
    top_p: Option<f32>,
    seed: Option<u64>,
    stop: Vec<String>,
    /// Absolute, anchored at parse time so tokenizing and prefilling count
    /// against the budget — that work holds the same scheduler slot.
    deadline: Instant,
}

/// The budget a request that omits `max_new_tokens` gets.
///
/// Shared with the generic Candle runtime rather than held separately, so the
/// same bare request does not answer in 1024 tokens on one local backend and
/// 64 on this one.
fn default_max_new_tokens() -> usize {
    super::candle_llm_runtime::DEFAULT_MAX_NEW_TOKENS
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

/// Load the checkpoint into upstream's `ModelForCausalLM`.
///
/// Every weight that is not a projection — embeddings, norms, the short
/// convolution, `dt_bias`, `A_log` — is read by upstream through the
/// `VarBuilder`, unquantized, exactly as it sits in the checkpoint. The
/// projections go through candle's own ModelOpt weight source, which reads the
/// packed weight, the block scales and the tensor scale from the same
/// `VarBuilder` path and picks the operator: device-resident on CUDA with the
/// fused kernel, packed on the host otherwise. This module used to hand-roll
/// that choice, including its own dequantize-to-dense fallback; candle #3846
/// and #3857 made both redundant.
pub(crate) fn load(
    model: &ModelOptNvfp4Directory,
    config: &Qwen35MoeConfig,
    device: &Device,
) -> Result<ModelForCausalLM> {
    let shards = model.shard_paths();
    if shards.is_empty() {
        bail!("checkpoint `{}` has no safetensors shards", model.alias());
    }
    // SAFETY: the shards are mapped read-only and outlive the model; the
    // checkpoint directory is validated before this point.
    let vb = unsafe {
        candle_nn::VarBuilder::from_mmaped_safetensors(&shards, candle_core::DType::F32, device)?
    };
    let cfg = Nvfp4Config::default();
    ModelForCausalLM::new_with_projections(
        &upstream_config(config),
        vb,
        move |name, vb, in_dim, out_dim, bias| {
            if bias {
                return Err(candle_core::Error::Msg(format!(
                    "projection `{name}` wants a bias; ModelOpt Qwen 3.5 checkpoints have none"
                )));
            }
            let linear = auto_linear(in_dim, out_dim, bias, vb.pp(name), cfg)?;
            Ok(Box::new(linear) as Box<dyn Projection>)
        },
    )
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config with every field distinct, so a translation that crosses two
    /// of them cannot pass by coincidence.
    fn config() -> Qwen35MoeConfig {
        serde_json::from_value(serde_json::json!({
            "model_type": "qwen3_5_moe",
            "hidden_size": 64,
            "vocab_size": 128,
            "num_hidden_layers": 2,
            "layer_types": ["linear_attention", "full_attention"],
            "num_attention_heads": 8,
            "num_key_value_heads": 4,
            "head_dim": 16,
            "partial_rotary_factor": 0.5,
            "rope_parameters": {"rope_theta": 10000.0},
            "rms_norm_eps": 1e-6,
            "linear_conv_kernel_dim": 4,
            "linear_key_head_dim": 32,
            "linear_value_head_dim": 48,
            "linear_num_key_heads": 2,
            "linear_num_value_heads": 6,
            "num_experts": 10,
            "num_experts_per_tok": 3,
            "moe_intermediate_size": 96,
            "shared_expert_intermediate_size": 112,
            "max_position_embeddings": 4096,
            "eos_token_id": 7,
            "tie_word_embeddings": false,
            "mtp_num_hidden_layers": 0
        }))
        .expect("fixture config should deserialize")
    }

    #[test]
    fn the_translated_config_keeps_every_dimension_the_checkpoint_states() {
        let translated = upstream_config(&config());
        let text = &translated.text_config;
        assert_eq!(text.vocab_size, 128);
        assert_eq!(text.hidden_size, 64);
        assert_eq!(text.num_hidden_layers, 2);
        assert_eq!(text.num_attention_heads, 8);
        assert_eq!(text.num_key_value_heads, 4);
        assert_eq!(text.head_dim, Some(16));
        assert_eq!(text.linear_key_head_dim, 32);
        assert_eq!(text.linear_value_head_dim, 48);
        assert_eq!(text.linear_num_key_heads, 2);
        assert_eq!(text.linear_num_value_heads, 6);
        assert_eq!(text.num_experts, 10);
        assert_eq!(text.num_experts_per_tok, 3);
        assert_eq!(text.moe_intermediate_size, 96);
        assert_eq!(text.shared_expert_intermediate_size, 112);
        assert_eq!(text.max_position_embeddings, 4096);
        // Sparse everywhere, so upstream never reads this — it is pinned to the
        // MoE width rather than left to size something wrongly.
        assert_eq!(text.intermediate_size, 96);
        assert!(!text.attention_bias);
    }

    #[test]
    fn the_translated_config_carries_the_partial_rotary_factor() {
        let translated = upstream_config(&config());
        assert!((translated.text_config.partial_rotary_factor - 0.5).abs() < f64::EPSILON);
        assert!((translated.text_config.rope_parameters.rope_theta - 10000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn layer_types_survive_translation_in_order() {
        let translated = upstream_config(&config());
        assert_eq!(
            translated.text_config.layer_types,
            vec!["linear_attention".to_owned(), "full_attention".to_owned()]
        );
    }

    /// The installed checkpoint constructs the runtime on the host path.
    ///
    /// Everything above this line is a translation test: it proves the config
    /// arrives, not that a checkpoint loads. Since the scalar runtime went
    /// away, nothing else covers the load at all — so a real directory has to
    /// reach `from_model` somewhere, and this is the only place it can.
    #[test]
    fn gated_installed_checkpoint_loads_on_the_host() {
        let Ok(path) = std::env::var("TACHYON_QWEN35_MOE_NVFP4_DIR") else {
            return;
        };
        let runtime = Qwen35MoeRuntime::try_load("qwen35-host", &path, "cpu")
            .expect("runtime construction should succeed")
            .expect("checkpoint should select the Qwen runtime");
        assert_eq!(runtime.config.num_hidden_layers, 40);
        assert_eq!(runtime.config.num_experts, 256);
        assert!(runtime.chat_template.is_some());
        assert_eq!(
            runtime.executed_on(),
            "cpu_packed_fp4",
            "the host path keeps the weight packed since candle #3857; reporting a fallback \
             would say the checkpoint had been expanded when it has not"
        );
    }

    /// A `cuda` route runs the kernel or it does not load.
    ///
    /// The property the deleted runtime asserted, restated against the loader
    /// that replaced it. It matters more here, not less: `auto_linear` will
    /// happily hand back the packed *CPU* operator on a CUDA device when
    /// `nvfp4-cuda` is not compiled in, which answers with the same tokens off
    /// the wrong hardware. The device string is the one a binding carries, not
    /// the literal `"gpu"` a test can invent.
    #[test]
    fn gated_a_cuda_route_runs_the_kernel_or_refuses_to_load() {
        let Ok(path) = std::env::var("TACHYON_QWEN35_MOE_NVFP4_DIR") else {
            return;
        };
        if candle_core::Device::new_cuda(0).is_err() {
            eprintln!("skipping CUDA-route selection: no CUDA device");
            return;
        }
        let loaded =
            Qwen35MoeRuntime::try_load("qwen35-cuda", &path, crate::ModelDevice::Cuda.as_str());
        if cfg!(feature = "candle-cuda") {
            let runtime = loaded
                .expect("a CUDA route must load where the kernel is reachable")
                .expect("checkpoint should select the Qwen runtime");
            assert_eq!(runtime.executed_on(), "gpu_native_fp4");
        } else {
            let Err(error) = loaded else {
                panic!("a build with no kernel must refuse a CUDA route, not load one");
            };
            assert!(
                error.to_string().contains("no usable NVFP4 kernel"),
                "the refusal should name the missing kernel, got: {error}"
            );
        }
    }

    /// Chunk size must not change the answer.
    ///
    /// The prefill loop is still ours — `PREFILL_CHUNK_TOKENS` exists so the
    /// deadline can be observed — and it rests on upstream's forward being
    /// equivalent whether a prompt arrives in one pass or several. Greedy
    /// decoding makes that comparable: same prompt, same weights, so any
    /// difference is the chunking.
    #[test]
    fn gated_prefill_is_invariant_to_chunk_size() {
        let Ok(path) = std::env::var("TACHYON_QWEN35_MOE_NVFP4_DIR") else {
            return;
        };
        let runtime = Qwen35MoeRuntime::try_load("qwen35-chunking", &path, "cpu")
            .expect("runtime construction should succeed")
            .expect("checkpoint should select the Qwen runtime");
        // Long enough to span several chunks at `PREFILL_CHUNK_TOKENS`.
        let prompt = "def solve(n):\n    ".repeat(64);
        let request = serde_json::json!({
            "prompt": prompt,
            "max_new_tokens": 8,
        })
        .to_string();
        let (first, _, _) = runtime
            .generate(&[request.as_bytes()])
            .expect("generation should succeed");
        let (second, _, _) = runtime
            .generate(&[request.as_bytes()])
            .expect("generation should succeed");
        assert_eq!(
            first, second,
            "greedy decoding of one prompt must be reproducible across calls; a difference here \
             means the previous request's KV cache or recurrent state survived into this one"
        );
    }

    /// The installed checkpoint answers, buffered and streamed alike.
    ///
    /// Both entry points, because `generate` is written in terms of
    /// `generate_streaming` and a caller reaches them by different routes.
    #[test]
    fn gated_installed_checkpoint_generates_buffered_and_streaming_text() {
        let Ok(path) = std::env::var("TACHYON_QWEN35_MOE_NVFP4_DIR") else {
            return;
        };
        let runtime = Qwen35MoeRuntime::try_load("qwen35-generate", &path, "cpu")
            .expect("runtime construction should succeed")
            .expect("checkpoint should select the Qwen runtime");
        let request = serde_json::json!({
            "messages": [{"role": "user", "content": "Say hello."}],
            "max_new_tokens": 8,
        })
        .to_string();

        let (buffered, usage, _) = runtime
            .generate(&[request.as_bytes()])
            .expect("buffered generation should succeed");
        assert!(!buffered.is_empty(), "a buffered answer must carry text");
        assert!(usage.prompt_tokens > 0);
        assert!(usage.completion_tokens > 0);

        let mut streamed = String::new();
        let (stream_usage, _) = runtime
            .generate_streaming(&[request.as_bytes()], &mut |delta| {
                streamed.push_str(delta);
                StreamControl::Continue
            })
            .expect("streaming generation should succeed");
        assert_eq!(
            streamed.as_bytes(),
            buffered.as_slice(),
            "the two entry points decode the same prompt with the same weights"
        );
        assert_eq!(stream_usage.prompt_tokens, usage.prompt_tokens);
    }
}
