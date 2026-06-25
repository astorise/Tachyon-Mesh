# Design: Wiring the Model-Parallel Engines into the Production Runtime

This change adds no new compute. Every engine it dispatches to (`TensorParallelLlama`, `PipelineParallelLlama`, `ExpertParallelMlp`) already exists and is numerically verified against a dense reference. The work is the *control flow* that selects them, the *config plumbing* that carries the strategy, and the *build/runtime activation* of the CUDA backend the engines were designed for.

## 1. The gap, precisely

```
apply-model-deployment        IntegrityModelBinding            LoadedModel               engine
(config-ai.wit)               (domain_types.rs)                (candle_llm_runtime.rs)
  hardware-strategy:    ──X──>  alias, path, device, qos   ──X──>  Safetensors        ──>  candle::Llama (dense)
   distribution-mode             (NO strategy fields)               Gguf               ──>  quantized Llama
   device-ids                                                       (NO Parallel)
   stage-layer-ranges
   expert-device-map      the two ──X── are the missing links this change adds
   pipeline-depth
```

The strategy is fully described in WIT and validated *structurally* inside the config-api Wasm guest (Task 3 of the distributed change), but it stops there: it is never carried into `IntegrityModelBinding`, and `try_load` has no branch that reads it. We add both links and the `LoadedModel::Parallel` variant they target.

## 2. Config plumbing (`IntegrityModelBinding`)

Add an optional, default-`single` strategy to the binding so existing serialized configs deserialize unchanged:

```rust
pub(crate) struct IntegrityModelBinding {
    pub(crate) alias: String,
    pub(crate) path: String,
    pub(crate) device: ModelDevice,
    pub(crate) qos: RouteQos,
    pub(crate) dynamic: bool,
    /// Mirrors wit/config-ai.wit `hardware-strategy`. Default = `single` with
    /// empty lists, which serializes to nothing (skip_serializing_if) and
    /// deserializes from older configs that predate the field.
    #[serde(default, skip_serializing_if = "HardwareStrategy::is_single")]
    pub(crate) hardware_strategy: HardwareStrategy,
}

pub(crate) struct HardwareStrategy {
    pub(crate) distribution_mode: GpuDistribution, // single | tensor | pipeline | expert
    pub(crate) device_ids: Vec<u32>,
    pub(crate) stage_layer_ranges: Vec<(u32, u32)>,
    pub(crate) expert_device_map: Vec<(u32, u32)>,
    pub(crate) pipeline_depth: u32,
}
```

`HardwareStrategy::is_single()` returns true for the default so the new field is invisible to every existing test fixture and on-disk config. `system-faas-config-api` already maps the WIT `hardware-strategy` into a `parallel_topology::ParallelExecutionPlan` for structural validation; here it also forwards the same fields onto the binding it emits.

## 3. Dispatch (`LoadedModel` + `try_load`)

`LoadedModel` gains one variant; the engines are boxed because they are large and only ever constructed on the parallel path:

```rust
enum LoadedModel {
    Safetensors { /* unchanged */ },
    Gguf { /* unchanged */ },
    Parallel(ParallelModel),
}

enum ParallelModel {
    Tensor { model: Box<TensorParallelLlama>, config: Config, eos_tokens: Vec<u32>, devices: Vec<Device> },
    Pipeline { model: Box<PipelineParallelLlama> },
    // No `Expert` variant: there is no full MoE model in the tree, only the
    // verified per-layer `ExpertParallelMlp` primitive. An `expert_parallelism`
    // strategy is validated and device-placed, then rejected at load with a
    // typed error until a Mixtral-style loader lands (see below).
}
```
(Boxed so the enum's largest variant stays small — `clippy::large_enum_variant`.)

`try_load` is threaded a `&HardwareStrategy` (new parameter from the `ai_inference.rs` call site, defaulting to single for the mock/ONNX/NVFP4 branches that don't use it). The branch is:

```rust
match strategy.distribution_mode {
    GpuDistribution::Single => { /* existing line-258 device check + Safetensors/Gguf path, untouched */ }
    mode => {
        // 1. Build the plan from the binding's strategy fields.
        let plan = ParallelExecutionPlan::from_strategy(strategy)?;
        // 2. Fail fast against *discovered* hardware (not just structural shape).
        let topology = discover_cluster_topology();
        parallel_topology::validate_parallel_topology(&plan, &topology)?; // TopologyError -> CandleLlmError
        // 3. Resolve candle Devices for device_ids (cuda_if_available under the CUDA feature, else Cpu).
        let devices = resolve_devices(&plan.device_ids)?;
        // 4. Construct the matching engine from the on-disk checkpoint.
        let model = match mode {
            tensor   => ParallelModel::Tensor { model: Box::new(TensorParallelLlama::load(vb, &cfg, &devices)?), .. },
            pipeline => ParallelModel::Pipeline { model: Box::new(PipelineParallelLlama::load(.., &plan.stage_layer_ranges, &devices)?) },
            expert   => return Err(/* typed: full MoE checkpoint loader not yet implemented */),
        };
        LoadedModel::Parallel(model)
    }
}
```

Generation (`forward`/decode) gets a parallel arm alongside the existing `Safetensors`/`Gguf` arms, delegating to the engine's `forward`. **Tensor** parallelism carries the existing `TensorParallelCache`, so it supports the full autoregressive decode loop. The **pipeline** arm is **prefill-only today** (the engine itself is prefill-only): generation returns a typed "decode not yet wired for pipeline parallelism" error rather than silently producing wrong output, while prefill logits are proven equal to the dense reference. **Expert** parallelism never constructs a model — there is no full MoE loader — so it is rejected at load with a typed error after the plan is validated and placed.

### Why the line-258 check moves rather than disappears
The `requested_device != "cpu"` rejection stays the correct behaviour on a **CUDA-less build**: without `candle-core/cuda`, asking for a GPU genuinely can't be served. The check becomes "reject a GPU device unless the CUDA backend is compiled in," so CPU/CI builds keep the exact same typed error and the parallel path on those builds runs on `Device::Cpu` stand-ins (as every existing parallel test already does).

## 4. CUDA build activation (`Cargo.toml` + `parallel.rs`)

The audit reported `nvfp4-cuda` doesn't enable candle CUDA — correct, but note a `candle-cuda` feature **already exists** and does enable `candle-core/cuda` + `candle-nn/cuda` + `candle-onnx/cuda` + `candle-transformers/cuda`. So the CUDA backend is already *activatable*; this change makes the dispatch, multi-GPU enumeration, and all-reduce key off it.

**Why not make `nvfp4-cuda` pull `candle-cuda`?** That was the first attempt, but it broke the standard feature matrix: CI builds an all-features combo (including `nvfp4-cuda`) on a non-CUDA runner, and pulling `candle-core/cuda` drags in `cudarc`, whose build script requires `nvcc`. `nvfp4-cuda` must therefore stay CPU-buildable. The CUDA activation lives entirely behind the pre-existing `candle-cuda` feature:

```toml
nvfp4-cuda = ["ai-inference"]   # unchanged — stays in the non-CUDA matrix
# candle-cuda (pre-existing) = ["ai-inference", "candle-core/cuda", ...]  — the CUDA activation
```

`default = ["ring"]` stays CUDA-free so the standard build and CI remain CPU-only and reproducible without a GPU toolchain; `candle-cuda` is built and clippy-checked by the dedicated `cuda-quality` CI job.

Two runtime changes behind `#[cfg(feature = "candle-cuda")]`:

1. **`discover_cluster_topology()`** enumerates every available CUDA ordinal (it already probes `cuda_if_available` per ordinal — the loop only ever returned one device because the feature was off) and reports real free VRAM per device via NVML (`nvml-wrapper`, new dep gated on the CUDA feature). Interconnect class (NVLink vs PCIe) is read from NVML topology where available, still defaulting to `Pcie` conservatively when unknown. Without the feature, behaviour is byte-for-byte today's single-CPU report.
2. **`RowParallelLinear::forward`** all-reduce: today a CPU-staged summation across shard partial-sums. Under `candle-cuda` with >1 real device it issues an NCCL all-reduce instead; the CPU summation remains the path for single-device, CPU, and CUDA-less builds. The numeric contract (output equals the dense reference within tolerance) is unchanged — NCCL sum and CPU sum compute the same reduction — so the existing equivalence tests remain the correctness oracle, and a new hardware-gated test asserts the NCCL path matches the CPU path on real GPUs.

## 5. Validation flow (two-layer, unchanged contract)

- **Structural** (already exists, Wasm guest, no hardware): `system-faas-config-api` validates plan *shape* — device-id counts, pipeline stage contiguity/coverage, expert-map bounds — at `apply-model-deployment`.
- **Hardware-aware** (this change, in `core-host` where real devices exist): `validate_parallel_topology(plan, discovered_topology)` runs inside `try_load` *before* any weights load, returning `TopologyError` (insufficient device count / incompatible interconnect / VRAM-per-shard-exceeded) mapped to a typed `CandleLlmError`. A failure aborts the load with no partial allocation.

## 6. Out of scope for this change
- Pipeline decode-time per-stage KV cache and real per-stage thread/process overlap (P1 follow-up; the engine is wired in its proven prefill-only form).
- Top-k (>1) MoE routing; the wired engine is the verified top-1 dispatch.
- ONNX/NVFP4 GPU forward passes (`gpu-accelerated-inference-execution`) — this change only activates the shared candle CUDA build and the multi-GPU enumeration/NCCL the parallel engines need.
- Any WIT schema change — the `hardware-strategy`/`parallel-execution-plan` records are consumed as-is.
