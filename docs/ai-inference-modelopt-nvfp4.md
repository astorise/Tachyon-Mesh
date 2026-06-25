# ModelOpt/NVFP4 Inference

Tachyon supports ModelOpt/NVFP4 tensor primitives and a versioned Qwen 3.5 MoE
text runtime. NVFP4 is a weight encoding, not a generic architecture marker:
unknown architectures fail closed.

## Supported Layout

The loader recognizes Hugging Face-style directories with `model.safetensors.index.json` and sharded `.safetensors` files. NVFP4 detection comes from `W4A16_NVFP4` metadata in `config.json` or `hf_quant_config.json`, or from ModelOpt tensor names such as `.weight_scale_2`.

Supported linear groups are:

- `weight`: packed FP4 bytes stored as `U8`
- `weight_scale`: FP8 E4M3 block scales
- `weight_scale_2`: scalar tensor-level scale
- `input_scale`: optional activation scale
- BF16/F16 tensors: preserved as passthrough tensors

Packed FP4 bytes must not be interpreted as `f32`.

## Fallback Dequantization

The pure Rust fallback can convert packed FP4 plus FP8 E4M3 scales into BF16 or
F32 dense tensors. Quantized matvec execution consumes packed FP8 or NVFP4
weights directly, keeping fallback memory bounded to the active operator. It
validates block size, packed shape, scale shape, nibble order, tensor scale, and
fallback memory limits.

Fallback can be estimated for eager full-model dequantization or for a layer window. If estimated host RAM or accelerator memory exceeds configured limits, the runtime rejects fallback instead of silently loading an unsafe dense tensor set.

Production limits are configured in bytes:

- `TACHYON_NVFP4_MAX_HOST_RAM_BYTES`
- `TACHYON_NVFP4_MAX_ACCELERATOR_BYTES`
- `TACHYON_NVFP4_NATIVE_REQUIRED=1` to reject fallback

## Native Kernel Requirements

Native NVFP4 execution is capability-gated. A backend must report:

- FP4-capable accelerator hardware
- Runtime availability for the accelerator stack
- Compiled NVFP4 dequant and matmul kernels

Without all three, Tachyon selects the BF16/F32 fallback when allowed by memory limits, or returns an unsupported native-execution error when native execution is required.

## CUDA/CUTLASS Build

The concrete native backend is behind the `nvfp4-cuda` feature. Standard builds do not require CUDA, NVCC, or CUTLASS. CI and `--all-features` builds may enable `nvfp4-cuda` without native inputs; in that case Tachyon compiles the Rust capability layer and reports native NVFP4 CUDA kernels unavailable.

To compile the native backend, set:

- `TACHYON_NVFP4_CUDA_HOME`, `CUDA_HOME`, or `CUDA_PATH`: CUDA toolkit root
- `TACHYON_CUTLASS_INCLUDE_DIR`: CUTLASS include directory
- `TACHYON_NVFP4_CUDA_ARCH`: optional NVCC architecture, default `sm_100a`
- `TACHYON_NVCC`: optional explicit `nvcc` path

Example:

```powershell
$env:CUDA_PATH='C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.0'
$env:TACHYON_CUTLASS_INCLUDE_DIR='C:\src\cutlass\include'
$env:TACHYON_NVFP4_CUDA_ARCH='sm_120'
cargo test -p core-host --features "ai-inference nvfp4-cuda" modelopt_nvfp4
```

The native source provides CUDA entrypoints for NVFP4 dequantization and an initial linear matmul kernel that consumes ModelOpt packed FP4 weights and FP8 E4M3 scales. CUTLASS headers are required to compile the native kernels so future block-scaled Tensor Core kernels can use the same ABI boundary.

The Qwen runtime selects the native packed path once per loaded model. Each
request records `executed_on` as one of `gpu_native_fp4`, `gpu_fallback`, or
`cpu_fallback`. WASI-NN ONNX execution records `gpu_onnx` or `cpu`. The bounded
in-process telemetry log is available through
`ai_inference::inference_execution_telemetry()` for admin/metrics consumers.

## Architecture Runtime

Detected ModelOpt/NVFP4 directories are registered as typed component sets and
never fall through to `MOCK_LLM_RESPONSE`. Checkpoints matching
`qwen3.5-moe-text-modelopt-0.44-v1` execute through the hybrid Qwen 3.5 runtime.
Other architectures return an actionable compatibility error.

The Qwen runtime pages only the current layer, selected routed experts, shared
expert, and required attention state. See
[`ai/qwen35-moe-nvfp4-reference.md`](ai/qwen35-moe-nvfp4-reference.md).
