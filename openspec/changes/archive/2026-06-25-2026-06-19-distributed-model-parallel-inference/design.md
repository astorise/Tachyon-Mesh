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
- A *stage* (a contiguous layer range assigned to one node) executes a real transformer-block forward locally — see Task 5's notes in `tasks.md` for why this reuses Task 4's tensor-parallel block (degenerated to single-device/dense) rather than `ai-layer-wise-inference`'s per-layer streaming primitive, which turned out to be a placeholder forward pass, not real transformer math.
- Activations between stage `N` and `N+1` cross a point-to-point transport implementing the `StageTransport` trait. **Correction**: `grpc-http2` is the FaaS guest-request HTTP/gRPC router, not a node-to-node mesh transport — there was nothing existing to reuse. Task 5 added a minimal real TCP-socket transport instead; gRPC/HTTP2 framing can replace its wire format later without changing the trait contract or callers.
- A bounded number of micro-batches are kept in flight per stage (configurable pipeline depth) to avoid stage idling, mirroring GPipe-style scheduling, while staying within the existing host-RAM KV-cache paging budget from `vram-optimization`.

## 4. Expert Parallelism (MoE)
- MoE checkpoints are detected by tensor name (`detect_expert_count`, Task 6) rather than a config flag, since expert count is a property of the checkpoint, not the deployment: a layer with no `.experts.` tensors is dense.
- The router (gate) layer runs once per forward pass; once each token's top-1 expert is selected, tokens are grouped **by expert id** (`ExpertParallelMlp::forward`, Task 6) and only the tokens routed to a given expert are gathered and shipped to that expert's device — avoiding all-to-all replication of dense weights. Top-1 (not top-k) is implemented; top-k routing is not in this change's verified surface.
- Falls back to the existing dense path (`TensorParallelMlp`/`TensorParallelBlock`) when the loaded checkpoint declares no expert tensors. **Caveat (same class as §2/§3)**: this is true today only because no live MoE checkpoint loader exists in `candle_llm_runtime.rs` yet — see Task 6's notes in `tasks.md`. `ExpertPlacementPlan: expert_id -> device` is implemented and reused unchanged from pre-existing scaffolding.

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
