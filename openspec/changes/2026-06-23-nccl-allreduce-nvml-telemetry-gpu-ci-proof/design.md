# Design: NCCL All-Reduce, NVML Telemetry, and GPU-Proof CI

## 1. Scope and relationship to prior work
This change sits entirely inside the surface area `2026-06-19-distributed-model-parallel-inference` and `9e72d1f` (#204) already built and wired:

```
parallel.rs                          this change
  RowParallelLinear::forward    →    real NCCL AllReduce (candle-cuda, >1 CUDA device)
    (CPU-staged sum today)           CPU-staged sum (fallback: no candle-cuda, or 1 device, or Device::Cpu)

  discover_cluster_topology     →    NVML nvmlDeviceGetMemoryInfo per CUDA device
    (free_vram_bytes: 0 always)      (falls back to 0 if NVML absent/uninitializable)
```

No public API of `RowParallelLinear`, `TensorParallelBlock`, `ParallelExecutionPlan`, or `ClusterTopology` changes shape; `free_vram_bytes` and the all-reduce's output are unchanged in *type*, only in how the value is produced.

## 2. NCCL all-reduce

### 2.1 Dependency
Add `cudarc`'s NCCL feature (or `nccl-sys` directly, whichever `candle_core`'s own CUDA backend already vendors/links — `candle_core::cuda_backend` already depends on `cudarc`, so reusing `cudarc::nccl` avoids a second NCCL binding in the dependency graph) behind the existing `candle-cuda` Cargo feature. No new feature flag is introduced; `candle-cuda` already implies "we are linking against the real CUDA toolchain" and is the correct gate.

### 2.2 Communicator lifecycle
`RowParallelLinear` does not currently own any persistent per-shard-group state — `forward` is called per layer per request and builds its reduction list ad hoc. A real NCCL all-reduce requires a `Comm` per participating device, created once (NCCL communicator init is expensive: it allocates and exchanges out-of-band rendezvous state) and reused across calls.

```rust
/// One tensor-parallel shard group's NCCL communicators, one per participating
/// CUDA device, created once at model-load time and reused for every
/// `RowParallelLinear::forward` call in that group's lifetime.
#[cfg(feature = "candle-cuda")]
pub(crate) struct NcclShardGroup {
    comms: Vec<cudarc::nccl::Comm>,
}
```

`TensorParallelBlock::load` (the existing per-stage/per-shard-group loader) constructs one `NcclShardGroup` — via `ncclCommInitAll` for the common single-process, multi-GPU-handle case this codebase already targets (`TensorParallelLlama`/`TensorParallelBlock` are single-process, multi-`Device` constructs; see `2026-06-19-distributed-model-parallel-inference`'s design.md §2) — and threads it into each `RowParallelLinear` it builds, mirroring how `TensorParallelCache` is already built once per model and threaded through layers.

### 2.3 `RowParallelLinear::forward` dispatch
```rust
fn all_reduce(&self, partials: Vec<Tensor>) -> CandleResult<Tensor> {
    #[cfg(feature = "candle-cuda")]
    if let Some(group) = &self.nccl_group {
        if partials.len() > 1 && partials.iter().all(|t| matches!(t.device(), Device::Cuda(_))) {
            return group.all_reduce_sum(partials, &self.reduce_device);
        }
    }
    // Fallback: host-staged manual sum (existing behavior) — used when
    // candle-cuda is not compiled in, when fewer than 2 CUDA devices
    // participate, or when running on Device::Cpu (existing CPU test paths).
    self.cpu_staged_sum(partials)
}
```
This preserves every existing test's behavior unchanged (`Device::Cpu`-only tests never touch the new branch) while giving CUDA builds with ≥2 GPUs a real collective.

### 2.4 Single-GPU CI reality
The `cuda-quality` runner verified by this change's authoring audit has exactly one physical GPU (`RTX 3060`). NCCL supports multiple ranks on the same physical device via `ncclCommInitAll`'s loopback path (each rank gets its own CUDA context/stream on the same device) — this is the mechanism the GPU-proof CI test (§4) uses to exercise a real `ncclAllReduce` call without requiring a multi-GPU runner. This is explicitly a CI-only accommodation: production tensor-parallel deployments validated by `validate_parallel_topology` already require ≥2 *distinct* `device_ids`, unaffected by this detail.

## 3. NVML VRAM telemetry

### 3.1 Dependency
Add `nvml-wrapper` (pure-Rust NVML bindings, dlopen-based — does not require linking against `libnvidia-ml.so` at build time, so it cannot break CPU-only or non-NVIDIA builds) behind `candle-cuda`.

### 3.2 `discover_cluster_topology` change
```rust
#[cfg(feature = "candle-cuda")]
fn free_vram_bytes(ordinal: u32) -> u64 {
    static NVML: OnceLock<Option<Nvml>> = OnceLock::new();
    let Some(nvml) = NVML.get_or_init(|| Nvml::init().ok()) else { return 0 };
    nvml.device_by_index(ordinal)
        .and_then(|d| d.memory_info())
        .map(|m| m.free)
        .unwrap_or(0)
}
```
`Nvml::init()` failing (driver/library not present, permissions, non-NVIDIA host) degrades to the existing `0` ("unknown"), never panics or fails the build — matching the existing `VramPerShardExceeded` validation's documented contract ("zero means not yet sized and is never rejected on VRAM grounds").

### 3.3 Fix the comment bug
The existing comment on the CUDA-enumeration loop:
```rust
// With the candle CUDA backend compiled in (`candle-cuda`, pulled by
// `nvfp4-cuda`), `cuda_if_available` actually opens devices...
```
is corrected to:
```rust
// With the `candle-cuda` Cargo feature compiled in (a separate, sibling
// feature to `nvfp4-cuda` — enabling `nvfp4-cuda` alone does NOT pull in
// `candle-cuda`; see core-host/Cargo.toml), `cuda_if_available` actually
// opens devices...
```

## 4. GPU-proof CI

### 4.1 What "proof" means here
`cuda-quality` today proves `cargo check`/`cargo clippy --features candle-cuda` succeed against the real CUDA toolchain on a runner with a real GPU. It runs no `cargo test`. This change adds a test invocation step to that job:
```yaml
- name: Run CUDA NCCL all-reduce proof
  run: cargo test -p core-host --features candle-cuda nccl_all_reduce_matches_cpu_staged_reference -- --nocapture
```

### 4.2 The test itself
A new `#[cfg(feature = "candle-cuda")]`-gated test in `parallel.rs`:
- Builds a 2-rank `NcclShardGroup` via `ncclCommInitAll` against the single available CUDA device (loopback ranks, §2.4).
- Constructs the same partial-sum tensors used by the existing `row_parallel_all_reduce_matches_single_device_reference` CPU test.
- Asserts the NCCL-reduced result matches the existing CPU-staged-sum reference within the same `1e-4` tolerance already used by that test.
- Is skipped (not merely passed) with an explicit log message if `Device::cuda_if_available(0)` reports no device, so the same test file remains compilable and runnable (as a no-op) on a `candle-cuda` build executed on a GPU-less host — distinct from `cuda-quality`'s runner, where it always executes.

### 4.3 Why not a brand-new CI job
The existing `cuda-quality` job already pays the cost of provisioning the self-hosted GPU runner, the CUDA toolchain discovery, and the `nvidia-smi`/`nvcc` verification steps (confirmed working in this change's authoring audit). Adding one `cargo test` invocation to that job is lower-risk and lower-maintenance than introducing a second GPU-runner job with its own provisioning steps to keep in sync.

## 5. Out of scope for this change
- Multi-node NCCL (communicator bootstrap across hosts via `ncclCommInitRank` + an out-of-band rendezvous service) — intra-node only, matching the existing tensor-parallel scope.
- NVML telemetry surfaced through `hardware-capabilities`'s MCP `hardware://` resources — this change only feeds `ai-inference`'s own `ClusterTopology.free_vram_bytes`.
- Replacing pipeline-parallel's `TcpStageTransport` with NCCL point-to-point send/recv — pipeline parallelism's activation hand-off is a separate transport already covered by `2026-06-19-distributed-model-parallel-inference`'s design and is out of scope here.
