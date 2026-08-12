# Qwen 3.5 MoE NVFP4 runtime reference

The runtime contract is based on the upstream Hugging Face implementation
`transformers/models/qwen3_5_moe/modeling_qwen3_5_moe.py` and NVIDIA ModelOpt
unified Hugging Face checkpoint metadata.

Initial compatibility profile:

- profile: `qwen3.5-moe-text-modelopt-0.44-v1`
- architecture: `Qwen3_5MoeForConditionalGeneration`
- outer model type: `qwen3_5_moe`
- text model type: `qwen3_5_moe_text`
- producer: `modelopt` version `0.44.0`
- weight graph: mixed `FP8` and `W4A16_NVFP4`
- NVFP4 group size: 16
- KV-cache declaration: `FP8`

The profile is fail-closed. A different producer version, layer type, routing
rule, quantization algorithm, or tensor contract requires a new versioned
profile and parity fixtures.

## Reference equations

Full attention uses normalized Q/K heads, partial rotary embedding, grouped KV
heads, causal scaled dot-product attention, a sigmoid output gate carried in
the doubled Q projection, then the output projection.

Linear-attention layers implement Qwen's gated delta rule:

1. Project Q/K/V and Z; apply depthwise causal convolution and SiLU.
2. L2-normalize Q/K and repeat key heads to the value-head count.
3. Compute `beta = sigmoid(b)` and
   `g = -exp(A_log) * softplus(a + dt_bias)`.
4. Decay the recurrent K/V state by `exp(g)`.
5. Correct the value by the state lookup at K, scaled by beta.
6. Rank-one update the recurrent state and read it at Q.
7. Apply RMS normalization followed by the SiLU Z gate and output projection.

Sparse MoE routing computes FP32 softmax probabilities, selects deterministic
top-k experts, renormalizes selected probabilities to sum to one, executes only
selected experts, and adds the sigmoid-gated shared expert. Each expert is
`down(silu(gate(x)) * up(x))`.

## Reusable Candle inventory

Candle provides dense tensor operations, RMSNorm, Qwen 3 full-attention
building blocks, Qwen 3 MoE routing, fused MoE infrastructure, sampling,
tokenization integration, and KV-cache examples. It also provides the Qwen 3.5
hybrid architecture itself — `candle_transformers::models::qwen3_5`, including
`Qwen3_5GatedDeltaNet` and the recurrent gated delta rule. `qwen35_upstream.rs`
executes on it: the local scalar reimplementation that used to sit beside it
has been deleted, along with the parity test that compared the two. That test
had never run — the GPU job has no checkpoint — so the equivalence was never
demonstrated and cannot now be demonstrated that way.

What remains a Tachyon responsibility is the ModelOpt mixed FP8/NVFP4 tensor
mapping: reading the quantized-layer metadata and feeding upstream's modules
the weights it expects.

Whether the production NVFP4 CUDA kernels are reachable is `candle-nvfp4-kernels`'
answer, not a compute capability Tachyon checks for itself. This page used to
say SM100 or newer; candle #3831 removed that floor after establishing that
nothing in the kernel needs FP4 tensor cores. A build that cannot reach the
kernel may use only bounded layer/operator fallback, and must reject execution
when configured host or accelerator memory limits would be exceeded.

Sources:

- https://github.com/huggingface/transformers/blob/main/src/transformers/models/qwen3_5_moe/modeling_qwen3_5_moe.py
- https://github.com/NVIDIA/TensorRT-Model-Optimizer/tree/main/examples/llm_ptq

Related tracking:

- Tachyon architecture parity: https://github.com/astorise/Tachyon-Mesh/issues/228
- Multimodal/VLM execution (not part of this text runtime):
  https://github.com/astorise/Tachyon-Mesh/issues/238

## Adding a compatible checkpoint

Do not register a checkpoint from its directory name or from the presence of
NVFP4 tensors alone. Add or extend a versioned descriptor only after validating
architecture identifiers, ordered layer semantics, routing behavior, producer
version, quantization assignments, and the exact tensor contract. A new
semantic variant also requires deterministic intermediate-state and logits
fixtures.

## Fixture regeneration and qualification

For the installed production checkpoint, set `TACHYON_QWEN35_MOE_NVFP4_DIR` and
run the gated tests in `qwen35_upstream.rs`. They are the only coverage the
loader has: no synthetic checkpoint exercises it, so a run without that
variable set skips them rather than substituting a weaker check.

The same name is the switch for the `GPU Acceptance` workflow, as a repository
variable pointing at the checkpoint on the GPU runner. Setting it is the whole
opt-in: unset, the workflow skips; set, it runs and fails if the path is not a
directory on the runner. Nothing there is gated on the hardware — candle #3831
dropped the FP4 tensor-core floor, so a device without the tensor cores takes
the fallback and answers the same tokens more slowly.

`scripts/export_qwen35_moe_fixtures.py` exported golden intermediate states and
logits from a local trusted tiny model, for the scalar runtime's parity tests to
compare against. Those tests were deleted with the runtime, so nothing reads its
output today. The script is kept — it is the starting point for any future
numeric validation against `transformers` — but running it currently produces a
file no test consumes.

Runtime controls:

- `TACHYON_MODEL_OPT_NVFP4_DIR`: generic ModelOpt parser probe.

`TACHYON_QWEN35_MAX_DENSE_OPERATOR_BYTES` and `TACHYON_QWEN35_WORKING_SET_BYTES`
bounded the scalar runtime's per-operator dequantization and its prepared-weight
working set. Neither exists now: candle holds the weights, packed, and there is
no working set to page. They are read by nothing, and setting them does nothing.

Contract failures identify the layer, expert, projection, tensor, shape, or
quantization assignment that must be corrected.
