//! The Qwen 3.5 MoE ModelOpt/NVFP4 compatibility profile.
//!
//! What a checkpoint has to *be* before anything will run it: the architecture
//! identifiers, the producer and its version, the quantization assignments,
//! the tensor contract, and the route a binding may ask for. Fail-closed by
//! construction — a semantic variant needs a new versioned profile, not a
//! looser check here.
//!
//! This was carved out of the scalar runtime it used to share a file with.
//! Execution belongs to candle now (`qwen35_upstream`); deciding what is
//! admissible does not, and never did.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::Value;

use super::{
    architecture_registry::{ArchitectureDescriptor, ArchitectureKind, ArchitectureMatch},
    modelopt_nvfp4::{
        ModelOptLinearTensors, ModelOptNvfp4Directory, SafetensorsDType, NVFP4_BLOCK_SIZE,
    },
};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Qwen35Route {
    /// The host does the multiplying: the dense path, by definition.
    Host,
    /// An accelerator does it. The NVFP4 kernel is CUDA-only, so this is the
    /// only accelerator spelling that can be honoured.
    Cuda,
}

impl Qwen35Route {
    pub(crate) fn parse(requested_device: &str) -> Result<Self> {
        match requested_device {
            "cpu" => Ok(Self::Host),
            // `cuda` is what a binding carries; `gpu` is the device-agnostic
            // spelling callers and tests use. Both are the same request.
            "cuda" | "gpu" => Ok(Self::Cuda),
            // Every other accelerator is refused rather than quietly served
            // from the host. `candle-nvfp4-kernels` has no backend for them,
            // and answering with the same tokens off the wrong device is the
            // silent descent this runtime exists to prevent.
            other => bail!(
                "Qwen 3.5 MoE runtime supports `cpu` or a CUDA accelerator (`cuda`/`gpu`), got \
                 `{other}`: the NVFP4 kernel has no {other} backend, and serving these weights \
                 from the host instead would answer with the same tokens and none of the meaning"
            ),
        }
    }

    fn is_accelerator(self) -> bool {
        self == Self::Cuda
    }
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

    /// The device string a binding actually carries must reach the kernel.
    ///

    /// `ModelDevice::as_str()` spells the accelerator `cuda`. The guard used
    /// to accept only `cpu` and `gpu`, so every CUDA manifest was refused
    /// before it reached the loader — the native NVFP4 path was unreachable
    /// outside the tests, which passed the literal `"gpu"` and stayed green.
    ///
    /// Asserting through `ModelDevice` rather than string literals is the
    /// point: a literal here would have agreed with the old guard and proved
    /// nothing.
    #[test]
    fn a_binding_device_selects_the_route_it_names() {
        assert_eq!(
            Qwen35Route::parse(crate::ModelDevice::Cuda.as_str()).expect("cuda binds a GPU route"),
            Qwen35Route::Cuda
        );
        assert_eq!(
            Qwen35Route::parse(crate::ModelDevice::Cpu.as_str()).expect("cpu binds a host route"),
            Qwen35Route::Host
        );
        // The device-agnostic spelling callers and gated tests use.
        assert_eq!(
            Qwen35Route::parse("gpu").expect("`gpu` is the same request as `cuda`"),
            Qwen35Route::Cuda
        );
    }

    /// Accelerators the NVFP4 kernel has no backend for are refused, not
    /// served from the host. A Metal binding that quietly answered on the CPU
    /// would return the same tokens off the wrong device, which is exactly
    /// what a `gpu` route is a claim against.
    #[test]
    fn an_accelerator_without_an_nvfp4_backend_is_refused_by_name() {
        for device in [
            crate::ModelDevice::Metal,
            crate::ModelDevice::Npu,
            crate::ModelDevice::Tpu,
        ] {
            let error = Qwen35Route::parse(device.as_str())
                .expect_err("no NVFP4 backend exists for this accelerator");
            assert!(
                error.to_string().contains(device.as_str()),
                "the refusal should name the device it refused, got: {error}"
            );
        }
    }
}
