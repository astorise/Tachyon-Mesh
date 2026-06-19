# Design: Distributed Model-Parallel Inference Engine

## 1. Relationship to existing capabilities
This engine is a new execution mode selected by the already-configured `hardware_strategy.distribution_mode` field (`wit/config-ai.wit`, capability `ai-orchestration`). It is consumed by `system-faas-model-broker` and executed by `core-host`'s Candle-backed inference path (capability `ai-inference`), reusing the layer-wise streaming primitives from `ai-layer-wise-inference` as the unit of work for each pipeline stage.

```
config-ai.wit (existing)        ai-inference (this change)        candle (vendor)
  distribution-mode:        →     ParallelExecutionPlan        →   Tensor/Device ops
  tensor-parallelism /             ├─ TensorShardPlan                across multiple
  pipeline-parallelism /           ├─ PipelineStagePlan              candle::Device
  expert-parallelism               └─ ExpertPlacementPlan            handles
```

## 2. Tensor Parallelism (intra-node)
- Column-parallel sharding for up/gate projections, row-parallel for down projections (Megatron-style), so only one all-reduce per transformer block is required.
- Each GPU in the node owns a `TensorShard { device: candle::Device, layer_range, shard_index, shard_count }`.
- Synchronization uses a host-local all-reduce (NCCL if compiled with `nvfp4-cuda`/CUDA, otherwise a CPU-staged reduce for correctness on non-NCCL builds) — must not block the existing single-GPU path when `multi_gpu: false`.

## 3. Pipeline Parallelism (cross-node)
- Reuses `ai-layer-wise-inference`'s per-layer mmap streaming as the local execution primitive for a *stage* (a contiguous layer range assigned to one node).
- Activations between stage `N` and `N+1` cross the existing mesh transport (gRPC/HTTP2 capability `grpc-http2`), not a new wire protocol.
- A bounded number of micro-batches are kept in flight per stage (configurable pipeline depth) to avoid stage idling, mirroring GPipe-style scheduling, while staying within the existing host-RAM KV-cache paging budget from `vram-optimization`.

## 4. Expert Parallelism (MoE)
- At load time, the broker partitions the checkpoint's expert tensors across the configured GPU/node set (`ExpertPlacementPlan: expert_id -> device`).
- The router (gate) layer runs on every node holding a model replica; once top-k experts are selected per token, only the tokens routed to a given expert are shipped to that expert's device — avoiding all-to-all replication of dense weights.
- Falls back to the existing dense path (no MoE) when the loaded checkpoint declares no expert tensors.

## 5. Topology validation (fail fast)
`apply-model-deployment` (existing `config-ai.wit` function) already returns `result<_, string>`. This change adds a validation pass invoked from that path:

```rust
enum TopologyError {
    InsufficientGpuCount { required: u32, available: u32 },
    IncompatibleInterconnect { required: InterconnectClass, available: InterconnectClass },
    VramPerShardExceeded { shard_vram_bytes: u64, gpu_vram_bytes: u64 },
}
```
A deployment requesting `tensor-parallelism`/`pipeline-parallelism`/`expert-parallelism` whose topology cannot be satisfied is rejected at `apply-model-deployment` time with a `TopologyError`, not silently downgraded to single-GPU.

## 6. WIT contract additions (`wit/ai/inference.wit`)
```wit
package tachyon:ai@1.1.0;

interface inference {
    // ... existing types ...

    enum parallel-strategy {
        none,
        tensor-parallel,
        pipeline-parallel,
        expert-parallel,
    }

    record parallel-execution-plan {
        strategy: parallel-strategy,
        device-ids: list<u32>,
        /// For pipeline-parallel: inclusive layer range per device, indexed by device-ids position.
        stage-layer-ranges: list<tuple<u32, u32>>,
        /// For expert-parallel: expert id -> device-ids index.
        expert-device-map: list<tuple<u32, u32>>,
        pipeline-depth: u32,
    }

    /// Validates that the requested plan can be satisfied by discovered hardware topology
    /// before any weights are loaded.
    validate-parallel-topology: func(plan: parallel-execution-plan) -> result<_, string>;
}
```

## 7. Out of scope for this change
- Cross-cloud-region pipeline stages (assumes same-cluster, low-latency interconnect).
- Dynamic re-sharding while a model is serving traffic (a topology is fixed at `apply-model-deployment` time; changing it requires redeploying the model).
