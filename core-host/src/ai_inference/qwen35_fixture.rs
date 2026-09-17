//! A complete Qwen 3.5 MoE NVFP4 checkpoint, small enough to write in a test.
//!
//! The gated tests beside the runtime need a real checkpoint, and the real one
//! is tens of gigabytes that CI is forbidden to download — so they were opt-in
//! on an installed directory, and every environment without that directory
//! covered the loader with nothing at all.
//!
//! Nothing in the load path wants the production geometry. `validate_semantics`
//! asks for consistency, not size; `validate_tensor_contract` derives every
//! expected shape from the config it was handed; and `load` builds the model
//! through upstream's projection factory off that same config. A checkpoint of
//! two layers and two experts therefore traverses the same code as one of forty
//! layers and two hundred and fifty-six.
//!
//! What this cannot carry is the claim that *the installed* checkpoint loads —
//! that stays with the gated tests, which still run when a directory is
//! configured. What it does carry is everything those tests prove about the
//! code: the contract, the packing, the forward pass, and the two entry points
//! agreeing.

use std::fs;
use std::path::Path;

use anyhow::Result;
use serde_json::{json, Value};

use super::modelopt_nvfp4::NVFP4_BLOCK_SIZE;

/// Hidden width. A multiple of `NVFP4_BLOCK_SIZE`, because it is the `K`
/// dimension of most operators and NVFP4 blocks along `K`.
const HIDDEN: usize = 64;
/// Matches the tiny tokenizer's word-level vocabulary exactly. A wider one
/// would let the model sample an id the tokenizer cannot decode, which reads as
/// a generation bug rather than as the fixture being wrong.
const VOCAB: usize = 4;
const LAYERS: usize = 2;
const HEADS: usize = 4;
const KV_HEADS: usize = 2;
const HEAD_DIM: usize = 16;
const LINEAR_KEY_HEADS: usize = 1;
const LINEAR_VALUE_HEADS: usize = 2;
const LINEAR_KEY_HEAD_DIM: usize = 16;
const LINEAR_VALUE_HEAD_DIM: usize = 16;
const LINEAR_CONV_KERNEL: usize = 4;
const EXPERTS: usize = 2;
const EXPERTS_PER_TOK: usize = 1;
const MOE_INTERMEDIATE: usize = 32;
const SHARED_EXPERT_INTERMEDIATE: usize = 32;
const MAX_POSITION_EMBEDDINGS: usize = 128;

/// `1.0` in E4M3: sign 0, exponent 0111 (bias 7), mantissa 000.
const FP8_ONE: u8 = 0x38;
/// `0.5` in E4M3: the exponent one below.
const FP8_HALF: u8 = 0x30;
/// Two E2M1 codes per byte. `2` is `1.0` and `1` is `0.5` in the block LUT, so
/// a packed row alternates two magnitudes rather than repeating one — a weight
/// matrix of a single value makes every expert identical and hides a routing
/// mistake behind a correct-looking answer.
const FP4_PAIR: u8 = 0x21;
const FP4_PAIR_ALT: u8 = 0x12;

/// The word-level tokenizer the other tiny fixtures use.
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

/// Layer kinds in order. The profile refuses a checkpoint that is not hybrid,
/// so the fixture has to carry one of each — which is also what makes it
/// exercise both attention implementations.
const LAYER_TYPES: [&str; LAYERS] = ["linear_attention", "full_attention"];

/// A tensor as safetensors wants it: name, dtype, shape, bytes.
type TensorEntry = (String, &'static str, Vec<usize>, Vec<u8>);

/// Write the checkpoint into `root`.
pub(crate) fn write_tiny_qwen35_nvfp4_fixture(root: &Path) -> Result<()> {
    fs::create_dir_all(root)?;
    fs::write(root.join("config.json"), config_json().to_string())?;
    fs::write(
        root.join("hf_quant_config.json"),
        hf_quant_config_json().to_string(),
    )?;
    fs::write(root.join("tokenizer.json"), TINY_TOKENIZER_JSON)?;
    fs::write(
        root.join("tokenizer_config.json"),
        tokenizer_config_json().to_string(),
    )?;

    let tensors = tensors();
    let shard = "model-00001-of-00001.safetensors";
    write_shard(&root.join(shard), &tensors);
    write_index(root, shard, &tensors)?;
    Ok(())
}

/// The text config the profile validates and the runtime translates.
fn config_json() -> Value {
    json!({
        "architectures": ["Qwen3_5MoeForConditionalGeneration"],
        "model_type": "qwen3_5_moe",
        "text_config": {
            "model_type": "qwen3_5_moe_text",
            "hidden_size": HIDDEN,
            "vocab_size": VOCAB,
            "num_hidden_layers": LAYERS,
            "layer_types": LAYER_TYPES,
            "num_attention_heads": HEADS,
            "num_key_value_heads": KV_HEADS,
            "head_dim": HEAD_DIM,
            "partial_rotary_factor": 0.5,
            "rope_parameters": {"rope_theta": 10000.0},
            "rms_norm_eps": 1e-6,
            "linear_conv_kernel_dim": LINEAR_CONV_KERNEL,
            "linear_key_head_dim": LINEAR_KEY_HEAD_DIM,
            "linear_value_head_dim": LINEAR_VALUE_HEAD_DIM,
            "linear_num_key_heads": LINEAR_KEY_HEADS,
            "linear_num_value_heads": LINEAR_VALUE_HEADS,
            "num_experts": EXPERTS,
            "num_experts_per_tok": EXPERTS_PER_TOK,
            "moe_intermediate_size": MOE_INTERMEDIATE,
            "shared_expert_intermediate_size": SHARED_EXPERT_INTERMEDIATE,
            "max_position_embeddings": MAX_POSITION_EMBEDDINGS,
            // One past the last id the head can emit, so the fixture has no
            // reachable end-of-sequence and every generation runs to
            // `max_new_tokens`. A reachable one would make the answer's length
            // depend on which token the weights happen to favour, and a test
            // asserting that an answer is non-empty would start failing for a
            // reason that has nothing to do with the code it covers.
            "eos_token_id": VOCAB,
            "tie_word_embeddings": false,
            "mtp_num_hidden_layers": 0
        },
        "quantization_config": {
            "quant_method": "modelopt",
            "quant_algo": "W4A16_NVFP4",
            "config_groups": {
                "group_0": {
                    "weights": {"num_bits": 4, "type": "float", "group_size": NVFP4_BLOCK_SIZE},
                    "targets": ["model.language_model.layers.0.mlp.experts"]
                }
            }
        }
    })
}

/// The producer and mixed-precision metadata the profile requires. Both
/// algorithms have to appear or the profile refuses the checkpoint as
/// single-precision, which is the shape a real ModelOpt export carries.
fn hf_quant_config_json() -> Value {
    json!({
        "producer": {"name": "modelopt", "version": "0.44.0"},
        "quantization": {
            "quant_algo": "MIXED_PRECISION",
            "kv_cache_quant_algo": "FP8",
            "quantized_layers": {
                "model.language_model.layers.0.linear_attn.in_proj_qkv": {
                    "quant_algo": "FP8"
                },
                "model.language_model.layers.1.self_attn.q_proj": {
                    "quant_algo": "FP8"
                },
                "model.language_model.layers.0.mlp.experts.0.gate_proj": {
                    "quant_algo": "W4A16_NVFP4",
                    "group_size": NVFP4_BLOCK_SIZE
                },
                "lm_head": {
                    "quant_algo": "W4A16_NVFP4",
                    "group_size": NVFP4_BLOCK_SIZE
                }
            }
        }
    })
}

/// A chat template, so the fixture proves the same thing the installed
/// checkpoint does: that one is found and parsed rather than defaulted away.
fn tokenizer_config_json() -> Value {
    json!({
        "chat_template":
            "{% for message in messages %}{{ message['role'] }}: {{ message['content'] }}\n\
             {% endfor %}assistant:",
        "eos_token": "<unk>"
    })
}

fn tensors() -> Vec<TensorEntry> {
    let mut out = Vec::new();
    out.push(f32_tensor(
        "model.language_model.embed_tokens.weight",
        vec![VOCAB, HIDDEN],
    ));
    out.push(norm_tensor("model.language_model.norm.weight", HIDDEN));
    push_nvfp4(&mut out, "lm_head", VOCAB, HIDDEN);

    for (layer, kind) in LAYER_TYPES.iter().enumerate() {
        let prefix = format!("model.language_model.layers.{layer}");
        out.push(norm_tensor(
            &format!("{prefix}.input_layernorm.weight"),
            HIDDEN,
        ));
        out.push(norm_tensor(
            &format!("{prefix}.post_attention_layernorm.weight"),
            HIDDEN,
        ));

        match *kind {
            "linear_attention" => {
                let key_dim = LINEAR_KEY_HEADS * LINEAR_KEY_HEAD_DIM;
                let value_dim = LINEAR_VALUE_HEADS * LINEAR_VALUE_HEAD_DIM;
                let conv_channels = key_dim * 2 + value_dim;
                out.push(f32_tensor(
                    &format!("{prefix}.linear_attn.A_log"),
                    vec![LINEAR_VALUE_HEADS],
                ));
                out.push(f32_tensor(
                    &format!("{prefix}.linear_attn.dt_bias"),
                    vec![LINEAR_VALUE_HEADS],
                ));
                out.push(f32_tensor(
                    &format!("{prefix}.linear_attn.conv1d.weight"),
                    vec![conv_channels, 1, LINEAR_CONV_KERNEL],
                ));
                out.push(norm_tensor(
                    &format!("{prefix}.linear_attn.norm.weight"),
                    LINEAR_VALUE_HEAD_DIM,
                ));
                for suffix in ["in_proj_a", "in_proj_b"] {
                    push_dense(
                        &mut out,
                        &format!("{prefix}.linear_attn.{suffix}"),
                        LINEAR_VALUE_HEADS,
                        HIDDEN,
                    );
                }
                push_fp8(
                    &mut out,
                    &format!("{prefix}.linear_attn.in_proj_qkv"),
                    conv_channels,
                    HIDDEN,
                );
                push_fp8(
                    &mut out,
                    &format!("{prefix}.linear_attn.in_proj_z"),
                    value_dim,
                    HIDDEN,
                );
                push_fp8(
                    &mut out,
                    &format!("{prefix}.linear_attn.out_proj"),
                    HIDDEN,
                    value_dim,
                );
            }
            _ => {
                for suffix in ["q_norm", "k_norm"] {
                    out.push(norm_tensor(
                        &format!("{prefix}.self_attn.{suffix}.weight"),
                        HEAD_DIM,
                    ));
                }
                push_fp8(
                    &mut out,
                    &format!("{prefix}.self_attn.q_proj"),
                    HEADS * HEAD_DIM * 2,
                    HIDDEN,
                );
                for suffix in ["k_proj", "v_proj"] {
                    push_fp8(
                        &mut out,
                        &format!("{prefix}.self_attn.{suffix}"),
                        KV_HEADS * HEAD_DIM,
                        HIDDEN,
                    );
                }
                push_fp8(
                    &mut out,
                    &format!("{prefix}.self_attn.o_proj"),
                    HIDDEN,
                    HEADS * HEAD_DIM,
                );
            }
        }

        push_dense(&mut out, &format!("{prefix}.mlp.gate"), EXPERTS, HIDDEN);
        push_dense(
            &mut out,
            &format!("{prefix}.mlp.shared_expert_gate"),
            1,
            HIDDEN,
        );
        for suffix in ["gate_proj", "up_proj"] {
            push_nvfp4(
                &mut out,
                &format!("{prefix}.mlp.shared_expert.{suffix}"),
                SHARED_EXPERT_INTERMEDIATE,
                HIDDEN,
            );
        }
        push_nvfp4(
            &mut out,
            &format!("{prefix}.mlp.shared_expert.down_proj"),
            HIDDEN,
            SHARED_EXPERT_INTERMEDIATE,
        );
        for expert in 0..EXPERTS {
            for suffix in ["gate_proj", "up_proj"] {
                push_nvfp4(
                    &mut out,
                    &format!("{prefix}.mlp.experts.{expert}.{suffix}"),
                    MOE_INTERMEDIATE,
                    HIDDEN,
                );
            }
            push_nvfp4(
                &mut out,
                &format!("{prefix}.mlp.experts.{expert}.down_proj"),
                HIDDEN,
                MOE_INTERMEDIATE,
            );
        }
    }
    out
}

/// A dense operator: one `.weight`, no scales. The absence of scales is what
/// makes the loader classify it as passthrough, so adding any would silently
/// change its kind.
fn push_dense(out: &mut Vec<TensorEntry>, base: &str, out_dim: usize, in_dim: usize) {
    out.push(f32_tensor(&format!("{base}.weight"), vec![out_dim, in_dim]));
}

/// An FP8 operator: E4M3 weights beside a scalar F32 scale.
fn push_fp8(out: &mut Vec<TensorEntry>, base: &str, out_dim: usize, in_dim: usize) {
    let bytes = (0..out_dim * in_dim)
        .map(|index| if index % 3 == 0 { FP8_HALF } else { FP8_ONE })
        .collect();
    out.push((
        format!("{base}.weight"),
        "F8_E4M3",
        vec![out_dim, in_dim],
        bytes,
    ));
    out.push(scalar_f32(&format!("{base}.weight_scale")));
}

/// An NVFP4 operator: packed nibbles, one E4M3 scale per block along `K`, and
/// one F32 scale for the tensor. The loader keys on `weight_scale_2` to tell
/// NVFP4 from FP8, and refuses an NVFP4 operator whose block scales are absent.
fn push_nvfp4(out: &mut Vec<TensorEntry>, base: &str, out_dim: usize, in_dim: usize) {
    let packed_cols = in_dim / 2;
    let packed = (0..out_dim * packed_cols)
        .map(|index| {
            if index % 2 == 0 {
                FP4_PAIR
            } else {
                FP4_PAIR_ALT
            }
        })
        .collect();
    out.push((
        format!("{base}.weight"),
        "U8",
        vec![out_dim, packed_cols],
        packed,
    ));
    let blocks = in_dim / NVFP4_BLOCK_SIZE;
    out.push((
        format!("{base}.weight_scale"),
        "F8_E4M3",
        vec![out_dim, blocks],
        vec![FP8_ONE; out_dim * blocks],
    ));
    out.push(scalar_f32(&format!("{base}.weight_scale_2")));
}

fn scalar_f32(name: &str) -> TensorEntry {
    (
        name.to_owned(),
        "F32",
        vec![1],
        1.0f32.to_le_bytes().to_vec(),
    )
}

/// Small, deterministic, and not all the same value: a constant matrix makes
/// every row of an attention or routing projection identical, which turns a
/// wrong permutation into a passing test.
fn f32_tensor(name: &str, shape: Vec<usize>) -> TensorEntry {
    let count: usize = shape.iter().product();
    let mut bytes = Vec::with_capacity(count * 4);
    for index in 0..count {
        let value = 0.02 * ((index % 7) as f32 - 3.0);
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    (name.to_owned(), "F32", shape, bytes)
}

/// RMSNorm scales are 1.0 so normalization is well conditioned; the fixture is
/// meant to produce a finite answer, not an interesting one.
fn norm_tensor(name: &str, width: usize) -> TensorEntry {
    let mut bytes = Vec::with_capacity(width * 4);
    for _ in 0..width {
        bytes.extend_from_slice(&1.0f32.to_le_bytes());
    }
    (name.to_owned(), "F32", vec![width], bytes)
}

fn write_shard(path: &Path, tensors: &[TensorEntry]) {
    let mut offset = 0usize;
    let mut header = serde_json::Map::new();
    let mut payload = Vec::new();
    for (name, dtype, shape, bytes) in tensors {
        let start = offset;
        offset += bytes.len();
        header.insert(
            name.clone(),
            json!({"dtype": dtype, "shape": shape, "data_offsets": [start, offset]}),
        );
        payload.extend_from_slice(bytes);
    }
    let header_bytes =
        serde_json::to_vec(&Value::Object(header)).expect("fixture header should serialize");
    let mut file = Vec::with_capacity(8 + header_bytes.len() + payload.len());
    file.extend_from_slice(&(header_bytes.len() as u64).to_le_bytes());
    file.extend_from_slice(&header_bytes);
    file.extend_from_slice(&payload);
    fs::write(path, file).expect("fixture shard should be written");
}

fn write_index(root: &Path, shard: &str, tensors: &[TensorEntry]) -> Result<()> {
    let total: usize = tensors.iter().map(|(_, _, _, bytes)| bytes.len()).sum();
    let weight_map = tensors
        .iter()
        .map(|(name, _, _, _)| (name.clone(), Value::String(shard.to_owned())))
        .collect::<serde_json::Map<_, _>>();
    let index = json!({
        "metadata": {"total_parameters": total, "total_size": total},
        "weight_map": weight_map
    });
    fs::write(
        root.join("model.safetensors.index.json"),
        serde_json::to_vec(&index)?,
    )?;
    Ok(())
}
