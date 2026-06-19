# Proposal: Tensor / Pipeline / Expert (MoE) Parallelism in the Inference Runtime

## Why
`openspec/specs/ai-inference/spec.md` already states a MUST: *"the orchestration configuration SHALL allow operators to define a `tensor_parallelism` strategy, forcing the underlying `wasi-nn` backend to partition model layers across multiple available GPUs"* (added by the archived change `2026-05-04-ai-orchestration-and-multi-gpu`). That change shipped the `wit/config-ai.wit` schema (`gpu-distribution` enum: `single | tensor-parallelism | pipeline-parallelism`) and the Tachyon-UI form to pick it, but it never shipped an engine. There are zero occurrences of `tensor_parallel`/`parallelism` in the Rust runtime today.

What Tachyon actually has:
- VRAM-aware **routing** of a whole request to a single node/GPU that holds the model (`vram-optimization`, `model-aware-routing`).
- Single-node **layer-wise streaming** (`ai-layer-wise-inference`): mmap weights in host RAM, stream one layer to one GPU at a time, drop it, repeat.
- **KV-cache** partitioning across the swarm (`kv-partition-v2`).

None of these split the *computation of a single forward pass* of one model instance across multiple GPUs or nodes. A model that does not fit on one GPU, or whose generation latency must be cut by splitting work across accelerators, cannot be served today — `apply-model-deployment` with `distribution-mode: tensor-parallelism` is accepted by the config API and then silently ignored by the engine.

## What Changes
Implement the missing runtime layer so the declared `gpu-distribution` strategy actually changes how a model executes:

1. **Tensor parallelism (intra-node)**: shard linear/attention layer weights (column/row-parallel) across the GPUs of a single node, with all-reduce/all-gather synchronization between shards on each layer.
2. **Pipeline parallelism (cross-node)**: assign contiguous layer ranges to different nodes/GPUs and stream activations between stages, reusing the existing layer-wise streaming machinery as the per-stage execution primitive instead of building a second engine.
3. **Expert parallelism (MoE)**: for mixture-of-experts checkpoints, place experts on distinct GPUs/nodes and route tokens to the GPU/node hosting the selected expert(s) per token, instead of replicating every expert everywhere.
4. **Topology validation**: at deployment time, reject (typed error) any `tensor-parallelism`/`pipeline-parallelism`/`expert-parallelism` strategy whose GPU/node topology requirement (count, interconnect, VRAM) cannot be satisfied by the cluster, instead of silently falling back to single-GPU.

This closes the gap between the existing `ai-orchestration` configuration surface (which already exists and is unchanged here) and the actual `candle`-backed execution engine in `core-host`/`system-faas-model-broker`.

## Non-Goals
- Does not change the `config-ai.wit` schema (already merged); this proposal only implements the engine behind the existing `distribution_mode` field.
- Does not implement heterogeneous accelerator parallelism (GPU+NPU mixed in one shard) — see the separate NPU/TPU proposal.
- Does not target training; this is inference-time parallelism only.

## Impact
- **Affected capability**: `ai-inference` (delta below). May warrant promoting to its own capability (`model-parallel-inference`) once implemented; kept as a delta on `ai-inference` for now to avoid spec sprawl before code exists.
- **Affected code**: `core-host` (wasi-nn bridge / batching scheduler), `system-faas-model-broker`, `wit/ai/inference.wit` (new parallel-execution options), `wit/config-ai.wit` (validation only, no schema change).
- **Risk**: cross-GPU/cross-node synchronization introduces new failure modes (partial shard load, NCCL/transport failures, stage timeout). Mitigated by the topology validation requirement (fail fast at deploy time) and by reusing the existing layer-wise streaming fault boundaries per stage.
