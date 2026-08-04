//! Loading a ModelOpt NVFP4 checkpoint into candle's own Qwen 3.5 layers.
//!
//! `qwen35_moe_runtime.rs` reimplements the architecture — attention, the
//! gated-delta recurrence, the sparse mixture, the norms — because for a long
//! time there was nowhere else for it to live: the upstream model was dense,
//! its projections were concrete `Linear`s, and NVFP4 weights could only be
//! multiplied through a host-staging kernel. Three changes removed all three
//! obstacles (candle #3831, #3832, #3837), and what is left for us to supply
//! is the only thing that was ever ours: how a `[rows, cols]` weight stored as
//! packed E2M1 nibbles multiplies an activation.
//!
//! So this module is a projection factory and a config translation, and the
//! model is upstream's.
//!
//! ## Why the scalar runtime is still here
//!
//! A resident NVFP4 operator needs a CUDA device that can execute FP4. Where
//! there is none — every CPU box, and today every CI runner — this loader
//! refuses rather than silently dequantizing a mixture-of-experts checkpoint to
//! f32, which would be eight times the packed size and would fit nowhere. The
//! scalar runtime stays as that host fallback. It is a second implementation of
//! one architecture and it will drift; it should be deleted once the GPU
//! pipeline actually runs.

use anyhow::{bail, Context, Result};
use candle_core::{DType, Device};
use candle_nn::VarBuilder;
use candle_transformers::models::qwen3_5::{
    Config as UpstreamConfig, ModelForCausalLM, Projection, RopeParameters as UpstreamRope,
    TextConfig,
};

use super::modelopt_nvfp4::{ModelOptLinearTensors, ModelOptNvfp4Directory};
use super::qwen35_moe_runtime::{LayerType, Qwen35MoeConfig};

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
/// * `hidden_act` — Qwen 3.5 is SwiGLU with SiLU, which is what our scalar
///   implementation applies and what the checkpoints carry.
/// * `attention_bias` — ModelOpt Qwen 3.5 checkpoints have no attention bias,
///   and our implementation never read one. If that ever changes the
///   projections will fail to find their bias tensors, which is the loud
///   failure rather than the silent one.
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

/// The checkpoint-relative name of a projection.
///
/// Upstream hands the factory a leaf name and a `VarBuilder` rooted at the
/// containing module, which is exactly the split a dense loader wants and
/// exactly the wrong one for us: an NVFP4 operator is three tensors that have
/// to be found by their full path. `VarBuilder::prefix` is the other half.
fn tensor_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}.{name}")
    }
}

/// One NVFP4 operator, as upstream's layers see it.
///
/// A pure pass-through, and deliberately so: the resident operator already
/// preserves the leading dimensions and never stages through the host, so
/// anything this wrapper did beyond delegating would be a copy the migration
/// exists to remove.
#[cfg(feature = "nvfp4-cuda")]
struct Nvfp4Projection {
    resident: super::modelopt_nvfp4::cuda::Nvfp4DeviceLinear,
}

#[cfg(feature = "nvfp4-cuda")]
impl Projection for Nvfp4Projection {
    fn forward(&self, xs: &candle_core::Tensor) -> candle_core::Result<candle_core::Tensor> {
        self.resident.forward(xs)
    }
}

/// How a projection's weight will be held on the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WeightResidency {
    /// Packed E2M1 nibbles, multiplied by the NVFP4 kernel. What the format is
    /// for, and the only form that fits a large checkpoint.
    Packed,
    /// Unpacked to dense f32 at load. Eight times the memory, and the only way
    /// to exercise these layers in a build that has no NVFP4 kernel.
    DequantizedF32,
}

/// The opt-in for the dequantized form. Off by default and deliberately
/// awkward to reach: it is a development tool, not a deployment mode.
const DEQUANTIZED_FALLBACK_ENV: &str = "TACHYON_QWEN35_DEQUANTIZED_FALLBACK";

/// Decide how this host will hold the weights, or refuse.
///
/// `native` is whether the NVFP4 kernel is reachable at all. It used to mean a
/// Blackwell device: the kernel gated itself on compute capability 10.0 even
/// though nothing in it needs FP4 tensor cores. candle #3831 separated the two
/// signals, so the packed path now runs wherever there is a CUDA device, and
/// the capability question moved to `supports_fp4_tensor_cores`.
///
/// What is left is the build. A `candle-cuda` binary compiled without the
/// `nvfp4-cuda` feature has a device and no kernel, and that is the one case
/// the dequantized form still answers.
pub(crate) fn residency(
    device_is_cuda: bool,
    native: bool,
    fallback_opted_in: bool,
) -> Result<WeightResidency> {
    if !device_is_cuda {
        bail!(
            "Qwen 3.5 through candle's layers needs a CUDA device; the scalar runtime is the \
             host path"
        );
    }
    if native {
        return Ok(WeightResidency::Packed);
    }
    if fallback_opted_in {
        return Ok(WeightResidency::DequantizedF32);
    }
    bail!(
        "this build has no NVFP4 kernel: it was compiled without the `nvfp4-cuda` feature. \
         Rebuild with it, or set {DEQUANTIZED_FALLBACK_ENV}=1 to unpack the weights to dense \
         f32 instead — eight times the memory of the packed checkpoint, which is why it is \
         not the default"
    )
}

fn fallback_opted_in() -> bool {
    std::env::var(DEQUANTIZED_FALLBACK_ENV).is_ok_and(|value| value == "1")
}

/// Build the projection named `path`, or explain why this host cannot.
fn projection(
    model: &ModelOptNvfp4Directory,
    path: &str,
    device: &Device,
    residency: WeightResidency,
) -> Result<Box<dyn Projection>> {
    let linear = model
        .modelopt_linear(path)
        .with_context(|| format!("projection `{path}` is not a ModelOpt linear"))?;
    let ModelOptLinearTensors::Nvfp4(linear) = linear else {
        bail!("projection `{path}` is not NVFP4-quantized; this loader has no dense path")
    };
    let packed = linear.packed_weight.tensor.read_bytes()?;
    let scales = linear.block_scales.tensor.read_bytes()?;
    let tensor_scale = super::modelopt_nvfp4::read_scalar_f32(&linear.tensor_scale.tensor)?;
    let shape = &linear.packed_weight.tensor.info.shape;
    let (rows, cols) = (shape[0], shape[1] * 2);

    match residency {
        WeightResidency::Packed => {
            #[cfg(feature = "nvfp4-cuda")]
            {
                // Uploaded onto the device the model's activations live on, not
                // whichever one this process happened to open first.
                let resident = super::modelopt_nvfp4::cuda::Nvfp4DeviceLinear::upload_on(
                    device,
                    &packed,
                    &scales,
                    tensor_scale,
                    rows,
                    cols,
                )
                .with_context(|| {
                    format!("projection `{path}` could not be made resident on a CUDA device")
                })?;
                Ok(Box::new(Nvfp4Projection { resident }))
            }
            #[cfg(not(feature = "nvfp4-cuda"))]
            {
                let _ = (packed, scales, tensor_scale, rows, cols, device);
                bail!("the packed NVFP4 path requires the `nvfp4-cuda` feature")
            }
        }
        WeightResidency::DequantizedF32 => {
            let dense = super::modelopt_nvfp4::dequantize_nvfp4_e4m3(
                &packed,
                &scales,
                tensor_scale,
                rows,
                cols,
                super::modelopt_nvfp4::Nvfp4OutputDType::F32,
            )
            .with_context(|| format!("projection `{path}` could not be dequantized"))?;
            let super::modelopt_nvfp4::Nvfp4DenseValues::F32(values) = dense else {
                bail!("dequantizing `{path}` to f32 produced another dtype")
            };
            let weight = candle_core::Tensor::from_vec(values, (rows, cols), device)?;
            Ok(Box::new(candle_nn::Linear::new(weight, None)))
        }
    }
}

/// Load the checkpoint into upstream's `ModelForCausalLM`.
///
/// Every weight that is not a projection — embeddings, norms, the short
/// convolution, `dt_bias`, `A_log` — is read by upstream through the
/// `VarBuilder`, unquantized, exactly as it sits in the checkpoint.
pub(crate) fn load(
    model: &ModelOptNvfp4Directory,
    config: &Qwen35MoeConfig,
    device: &Device,
) -> Result<ModelForCausalLM> {
    #[cfg(feature = "nvfp4-cuda")]
    let native = super::modelopt_nvfp4::cuda::Nvfp4CudaBackend::is_available();
    // Without the feature there is no kernel to be capable of anything, so the
    // dequantized form is the only one on offer — and still only on request.
    #[cfg(not(feature = "nvfp4-cuda"))]
    let native = false;
    let residency = residency(device.is_cuda(), native, fallback_opted_in())?;
    if residency == WeightResidency::DequantizedF32 {
        tracing::warn!(
            alias = model.alias(),
            "loading Qwen 3.5 with dequantized f32 weights: this device cannot execute the \
             NVFP4 kernel. Memory use is eight times the packed checkpoint and throughput is \
             not representative — for parity checks, not for serving"
        );
    }
    let shards = model.shard_paths();
    if shards.is_empty() {
        bail!("checkpoint `{}` has no safetensors shards", model.alias());
    }
    // SAFETY: the shards are mapped read-only and outlive the model; the
    // checkpoint directory is validated before this point.
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&shards, DType::F32, device)? };
    ModelForCausalLM::new_with_projections(
        &upstream_config(config),
        vb,
        |name, vb, _in_dim, _out_dim, bias| {
            if bias {
                return Err(candle_core::Error::Msg(format!(
                    "projection `{name}` wants a bias; ModelOpt Qwen 3.5 checkpoints have none"
                )));
            }
            projection(model, &tensor_path(&vb.prefix(), name), device, residency)
                .map_err(|error| candle_core::Error::Msg(format!("{error:#}")))
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
            "eos_token_id": 7,
            "max_position_embeddings": 256
        }))
        .expect("sample config deserializes")
    }

    #[test]
    fn the_translated_config_keeps_every_dimension_the_checkpoint_states() {
        let ours = config();
        let theirs = upstream_config(&ours).text_config;
        assert_eq!(theirs.vocab_size, ours.vocab_size);
        assert_eq!(theirs.hidden_size, ours.hidden_size);
        assert_eq!(theirs.num_hidden_layers, ours.num_hidden_layers);
        assert_eq!(theirs.num_attention_heads, ours.num_attention_heads);
        assert_eq!(theirs.num_key_value_heads, ours.num_key_value_heads);
        assert_eq!(theirs.head_dim, Some(ours.head_dim));
        assert_eq!(theirs.num_experts, ours.num_experts);
        assert_eq!(theirs.num_experts_per_tok, ours.num_experts_per_tok);
        assert_eq!(theirs.moe_intermediate_size, ours.moe_intermediate_size);
        assert_eq!(
            theirs.shared_expert_intermediate_size,
            ours.shared_expert_intermediate_size
        );
        assert_eq!(theirs.linear_conv_kernel_dim, ours.linear_conv_kernel_dim);
        assert_eq!(theirs.linear_key_head_dim, ours.linear_key_head_dim);
        assert_eq!(theirs.linear_value_head_dim, ours.linear_value_head_dim);
        assert_eq!(theirs.linear_num_key_heads, ours.linear_num_key_heads);
        assert_eq!(theirs.linear_num_value_heads, ours.linear_num_value_heads);
    }

    /// The rotary factor is the field that fails silently when it is dropped:
    /// upstream would rotate the whole head and the model would simply be
    /// wrong. It has to survive translation exactly.
    #[test]
    fn the_translated_config_carries_the_partial_rotary_factor() {
        let mut ours = config();
        ours.partial_rotary_factor = 0.25;
        assert_eq!(
            upstream_config(&ours).text_config.partial_rotary_factor,
            0.25
        );
    }

    #[test]
    fn layer_types_survive_translation_in_order() {
        let mut ours = config();
        ours.layer_types = vec![
            LayerType::LinearAttention,
            LayerType::FullAttention,
            LayerType::LinearAttention,
        ];
        assert_eq!(
            upstream_config(&ours).text_config.layer_types,
            vec!["linear_attention", "full_attention", "linear_attention"]
        );
    }

    #[test]
    fn a_projection_path_is_its_module_prefix_and_its_leaf() {
        assert_eq!(
            tensor_path("model.language_model.layers.3.mlp", "experts.5.gate_proj"),
            "model.language_model.layers.3.mlp.experts.5.gate_proj"
        );
        assert_eq!(
            tensor_path("model.language_model.layers.0.self_attn", "q_proj"),
            "model.language_model.layers.0.self_attn.q_proj"
        );
        // The head sits at the checkpoint root, where upstream's `VarBuilder`
        // has no prefix to contribute.
        assert_eq!(tensor_path("", "lm_head"), "lm_head");
    }

    #[test]
    fn a_host_with_the_kernel_holds_the_weights_packed() {
        assert_eq!(
            residency(true, true, false).expect("a capable device loads"),
            WeightResidency::Packed
        );
        // The opt-in does not override a device that can do the real thing.
        assert_eq!(
            residency(true, true, true).expect("a capable device loads"),
            WeightResidency::Packed
        );
    }

    /// The default when there is no kernel is to stop, not to quietly use
    /// eight times the memory. Said out loud, with both ways out in the
    /// message, because a silent 8x is how a machine dies at 3am.
    #[test]
    fn a_build_without_the_kernel_refuses_unless_asked() {
        let error =
            residency(true, false, false).expect_err("a build with no kernel cannot run packed");
        let message = error.to_string();
        assert!(message.contains("nvfp4-cuda"), "{message}");
        assert!(message.contains(DEQUANTIZED_FALLBACK_ENV), "{message}");
        assert_eq!(
            residency(true, false, true).expect("the opt-in allows the dense form"),
            WeightResidency::DequantizedF32
        );
    }

    #[test]
    fn a_host_without_a_cuda_device_is_refused_whatever_the_opt_in_says() {
        for opted_in in [false, true] {
            let error = residency(false, false, opted_in)
                .expect_err("a CPU host has nothing to offer these layers");
            assert!(
                error.to_string().contains("CUDA device"),
                "unexpected error: {error}"
            );
        }
    }
}
