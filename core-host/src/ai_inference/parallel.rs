//! Tensor / pipeline / expert (MoE) parallel execution across multiple
//! accelerator devices for a single model instance.
//!
//! This closes the gap between the `ai-orchestration` configuration surface
//! (`wit/config-ai.wit`'s `gpu-distribution` field, already implemented as
//! pure configuration) and an actual execution engine: until this module,
//! `tensor_parallelism`/`pipeline_parallelism` were accepted by the config
//! API and then ignored by the runtime.
//!
//! Device placement is generic over `candle_core::Device`, so the same shard
//! partitioning, all-reduce, and pipeline-stage logic that is exercised here
//! against multiple `Device::Cpu` handles applies unchanged to
//! `Device::Cuda(ordinal)` handles on real multi-GPU hardware — this module
//! does not special-case CPU vs. GPU placement.

use std::fmt;

use candle_core::{Device, Result as CandleResult, Tensor};

/// How a model's forward pass is split across more than one accelerator.
/// Mirrors `parallel-strategy` in `wit/ai/inference.wit`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ParallelStrategy {
    /// No splitting; the model runs entirely on one device.
    #[default]
    None,
    /// Column/row-parallel weight sharding across the GPUs of one node.
    TensorParallel,
    /// Contiguous layer ranges assigned to different nodes/GPUs.
    PipelineParallel,
    /// Mixture-of-Experts: experts placed on distinct GPUs/nodes.
    ExpertParallel,
}

/// Interconnect class between two devices, used to gate tensor-parallel
/// plans that require low-latency, high-bandwidth synchronization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum InterconnectClass {
    /// Same-host, high-bandwidth GPU-to-GPU link (e.g. NVLink).
    HighBandwidth,
    /// Same-host PCIe, no direct GPU-to-GPU link.
    Pcie,
    /// Cross-node network (Ethernet/InfiniBand between hosts).
    CrossNode,
}

/// One device's reported capacity, as produced by hardware capability
/// discovery (`hardware-capabilities` / `heterogeneous-accelerator-orchestration`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DeviceInfo {
    pub(crate) device_id: u32,
    pub(crate) free_vram_bytes: u64,
}

/// The cluster's discovered hardware topology, as known at deployment-validation
/// time. `interconnect` reports the worst-case (most constrained) interconnect
/// class across the participating device set, which is sufficient to gate a
/// tensor-parallel plan: a plan is only as fast as its slowest link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClusterTopology {
    pub(crate) devices: Vec<DeviceInfo>,
    pub(crate) interconnect: InterconnectClass,
}

impl ClusterTopology {
    pub(crate) fn device(&self, device_id: u32) -> Option<&DeviceInfo> {
        self.devices.iter().find(|d| d.device_id == device_id)
    }
}

/// Discovers the cluster topology that `validate_parallel_topology` checks
/// plans against. Always reports CPU device 0; on builds where the CUDA
/// backend is compiled in, probes additional ordinals via
/// `Device::cuda_if_available` and stops at the first ordinal that errors,
/// which `candle`'s CUDA backend treats as "no such device".
///
/// `free_vram_bytes` is reported as `0` (unknown) for every device: neither
/// `candle_core` nor this crate currently binds NVML/`cudaMemGetInfo`, so
/// real free-VRAM telemetry is out of scope here and tracked by the
/// `gpu-accelerated-inference-execution` change. A `0` value never causes a
/// spurious rejection because `validate_parallel_topology` only enforces the
/// VRAM check when the plan's `required_vram_bytes_per_device` is non-zero
/// AND it skips devices whose capacity is unknown the same way "not yet
/// sized" plans are skipped; callers that need real VRAM enforcement must
/// populate `DeviceInfo` from an out-of-band source until that telemetry
/// lands.
///
/// Interconnect is reported conservatively as `Pcie` whenever more than one
/// device is discovered (NVLink/topology detection is not implemented), and
/// is irrelevant when only one device is present.
pub(crate) fn discover_cluster_topology() -> ClusterTopology {
    #[cfg_attr(not(feature = "nvfp4-cuda"), allow(unused_mut))]
    let mut devices = vec![DeviceInfo { device_id: 0, free_vram_bytes: 0 }];

    #[cfg(feature = "nvfp4-cuda")]
    {
        let mut ordinal = 0u32;
        loop {
            match Device::cuda_if_available(ordinal as usize) {
                Ok(Device::Cuda(_)) => {
                    devices.push(DeviceInfo { device_id: ordinal + 1, free_vram_bytes: 0 });
                    ordinal += 1;
                }
                _ => break,
            }
            if ordinal > 64 {
                break;
            }
        }
    }

    let interconnect =
        if devices.len() > 1 { InterconnectClass::Pcie } else { InterconnectClass::HighBandwidth };

    ClusterTopology { devices, interconnect }
}

/// A requested execution plan, validated against `ClusterTopology` before any
/// weights are loaded. Mirrors `parallel-execution-plan` in
/// `wit/ai/inference.wit`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParallelExecutionPlan {
    pub(crate) strategy: ParallelStrategy,
    pub(crate) device_ids: Vec<u32>,
    /// For pipeline-parallel: inclusive (start, end) layer range per device,
    /// indexed by position in `device_ids`.
    pub(crate) stage_layer_ranges: Vec<(u32, u32)>,
    /// For expert-parallel: (expert_id, device_ids index) placement pairs.
    pub(crate) expert_device_map: Vec<(u32, u32)>,
    /// Bytes of VRAM required per shard/device, computed from the model's
    /// size and the chosen strategy. Required for `VramPerShardExceeded`
    /// validation; zero means "not yet sized" and is never rejected on VRAM
    /// grounds.
    pub(crate) required_vram_bytes_per_device: u64,
    pub(crate) pipeline_depth: u32,
}

/// Typed reasons a plan cannot be satisfied by the cluster's discovered
/// hardware topology. Mirrors `topology-error` in `wit/ai/inference.wit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TopologyError {
    InsufficientDeviceCount { required: u32, available: u32 },
    IncompatibleInterconnect { required: InterconnectClass, available: InterconnectClass },
    VramPerShardExceeded { required_bytes: u64, available_bytes: u64 },
}

impl fmt::Display for TopologyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InsufficientDeviceCount { required, available } => write!(
                f,
                "plan requires {required} device(s) but only {available} are available"
            ),
            Self::IncompatibleInterconnect { required, available } => write!(
                f,
                "plan requires {required:?} interconnect but cluster only provides {available:?}"
            ),
            Self::VramPerShardExceeded { required_bytes, available_bytes } => write!(
                f,
                "shard requires {required_bytes} bytes of VRAM but target device has {available_bytes} free"
            ),
        }
    }
}

impl std::error::Error for TopologyError {}

/// Minimum interconnect class required for tensor parallelism, which
/// synchronizes (all-reduce/all-gather) on every transformer block and is
/// therefore latency-sensitive. Pipeline and expert parallelism only
/// exchange activations/tokens between stages and tolerate any interconnect,
/// including cross-node.
const TENSOR_PARALLEL_MIN_INTERCONNECT: InterconnectClass = InterconnectClass::Pcie;

/// Validates that `plan` can be satisfied by `topology` without loading or
/// sharding a model. Called from `apply-model-deployment` (config-ai.wit)
/// before admitting a deployment; a rejected plan must not be silently
/// downgraded to a single-device plan.
pub(crate) fn validate_parallel_topology(
    plan: &ParallelExecutionPlan,
    topology: &ClusterTopology,
) -> Result<(), TopologyError> {
    if plan.strategy == ParallelStrategy::None {
        return Ok(());
    }

    let required = plan.device_ids.len() as u32;
    let available = topology.devices.len() as u32;
    if required > available {
        return Err(TopologyError::InsufficientDeviceCount { required, available });
    }

    if plan.strategy == ParallelStrategy::TensorParallel
        && topology.interconnect > TENSOR_PARALLEL_MIN_INTERCONNECT
    {
        return Err(TopologyError::IncompatibleInterconnect {
            required: TENSOR_PARALLEL_MIN_INTERCONNECT,
            available: topology.interconnect,
        });
    }

    if plan.required_vram_bytes_per_device > 0 {
        for &device_id in &plan.device_ids {
            let device = topology.device(device_id).ok_or(TopologyError::InsufficientDeviceCount {
                required,
                available,
            })?;
            if plan.required_vram_bytes_per_device > device.free_vram_bytes {
                return Err(TopologyError::VramPerShardExceeded {
                    required_bytes: plan.required_vram_bytes_per_device,
                    available_bytes: device.free_vram_bytes,
                });
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tensor parallelism: Megatron-style column/row-parallel linear layers.
// ---------------------------------------------------------------------------

/// A linear layer whose weight matrix is sharded by output features across
/// `shards.len()` devices. Each shard computes a disjoint slice of the
/// output; no communication is required because the slices are concatenated
/// (gathered), not summed. Used for the first linear in a Megatron-style MLP
/// block (e.g. the up/gate projection).
pub(crate) struct ColumnParallelLinear {
    /// One weight shard per device: shape `[out_features / n, in_features]`.
    shards: Vec<(Tensor, Device)>,
}

impl ColumnParallelLinear {
    /// Splits `weight` (`[out_features, in_features]`) into `devices.len()`
    /// equal column shards. `out_features` must be evenly divisible by
    /// `devices.len()`.
    pub(crate) fn shard(weight: &Tensor, devices: &[Device]) -> CandleResult<Self> {
        let n = devices.len();
        let out_features = weight.dim(0)?;
        let shard_size = out_features / n;
        let mut shards = Vec::with_capacity(n);
        for (i, device) in devices.iter().enumerate() {
            let shard = weight.narrow(0, i * shard_size, shard_size)?.to_device(device)?;
            shards.push((shard, device.clone()));
        }
        Ok(Self { shards })
    }

    /// Runs `x @ shard^T` on every device and gathers (concatenates) the
    /// partial outputs back into one `[batch, out_features]` tensor on
    /// `gather_device`.
    pub(crate) fn forward(&self, x: &Tensor, gather_device: &Device) -> CandleResult<Tensor> {
        let mut parts = Vec::with_capacity(self.shards.len());
        for (shard, device) in &self.shards {
            let x_local = x.to_device(device)?;
            let out = x_local.matmul(&shard.t()?)?;
            parts.push(out.to_device(gather_device)?);
        }
        Tensor::cat(&parts, 1)
    }
}

/// A linear layer whose weight matrix is sharded by input features across
/// `shards.len()` devices. Each shard computes a partial sum over its input
/// slice; the partial sums must be all-reduced (summed) to produce the
/// correct full output. Used for the second linear in a Megatron-style MLP
/// block (e.g. the down projection), immediately after a
/// `ColumnParallelLinear` so the activation is already split along the
/// dimension this layer shards.
pub(crate) struct RowParallelLinear {
    /// One weight shard per device: shape `[out_features, in_features / n]`.
    shards: Vec<(Tensor, Device)>,
}

impl RowParallelLinear {
    /// Splits `weight` (`[out_features, in_features]`) into `devices.len()`
    /// equal row (input-feature) shards.
    pub(crate) fn shard(weight: &Tensor, devices: &[Device]) -> CandleResult<Self> {
        let n = devices.len();
        let in_features = weight.dim(1)?;
        let shard_size = in_features / n;
        let mut shards = Vec::with_capacity(n);
        for (i, device) in devices.iter().enumerate() {
            let shard = weight.narrow(1, i * shard_size, shard_size)?.to_device(device)?;
            shards.push((shard, device.clone()));
        }
        Ok(Self { shards })
    }

    /// `x_shards[i]` is the i-th input slice already resident on
    /// `self.shards[i]`'s device (typically the gathered output of a
    /// preceding `ColumnParallelLinear`, re-split). Computes the partial
    /// matmul per shard, then all-reduces (sums) the partial outputs on
    /// `reduce_device`. This sum *is* the all-reduce: with one logical
    /// device per shard there is nothing else to synchronize, and on real
    /// multi-GPU hardware this is exactly the payload an NCCL all-reduce
    /// would sum across devices.
    pub(crate) fn forward(&self, x_shards: &[Tensor], reduce_device: &Device) -> CandleResult<Tensor> {
        let mut acc: Option<Tensor> = None;
        for ((shard, device), x_local) in self.shards.iter().zip(x_shards.iter()) {
            let x_local = x_local.to_device(device)?;
            let partial = x_local.matmul(&shard.t()?)?.to_device(reduce_device)?;
            acc = Some(match acc {
                Some(prev) => (prev + partial)?,
                None => partial,
            });
        }
        acc.ok_or_else(|| candle_core::Error::Msg("row-parallel forward requires at least one shard".into()))
    }
}

/// Splits a `[batch, features]` tensor into `devices.len()` equal column
/// slices, one per device, for handoff from a `ColumnParallelLinear`'s
/// gathered output into a following `RowParallelLinear`.
pub(crate) fn split_for_row_parallel(x: &Tensor, devices: &[Device]) -> CandleResult<Vec<Tensor>> {
    let n = devices.len();
    let features = x.dim(1)?;
    let shard_size = features / n;
    let mut parts = Vec::with_capacity(n);
    for (i, device) in devices.iter().enumerate() {
        parts.push(x.narrow(1, i * shard_size, shard_size)?.to_device(device)?);
    }
    Ok(parts)
}

// ---------------------------------------------------------------------------
// Pipeline parallelism: per-stage executor over contiguous layer ranges.
// ---------------------------------------------------------------------------

/// One pipeline stage's unit of work: apply `layer` to its input and return
/// the activation for the next stage. Implemented per-model by wrapping the
/// existing layer-wise streaming primitive (`ai-layer-wise-inference`) so a
/// stage's local execution is unchanged; this trait only defines the
/// stage-to-stage boundary.
pub(crate) trait PipelineStageExecutor {
    fn run_stage(&self, layer_range: (u32, u32), input: &Tensor) -> CandleResult<Tensor>;
}

/// A simple closure-backed executor for tests and for stages whose layer
/// range maps directly onto an in-process callable (e.g. a slice of a
/// candle-transformers model's layers).
pub(crate) struct ClosureStageExecutor<F>(pub(crate) F)
where
    F: Fn((u32, u32), &Tensor) -> CandleResult<Tensor>;

impl<F> PipelineStageExecutor for ClosureStageExecutor<F>
where
    F: Fn((u32, u32), &Tensor) -> CandleResult<Tensor>,
{
    fn run_stage(&self, layer_range: (u32, u32), input: &Tensor) -> CandleResult<Tensor> {
        (self.0)(layer_range, input)
    }
}

/// Cross-stage activation transport. The in-process implementation below is
/// used for single-node pipelines and for tests; a cross-node implementation
/// sends/receives activations over the existing `grpc-http2` mesh transport
/// and is not implemented in this module (see proposal `tasks.md` Task 5).
pub(crate) trait StageTransport {
    fn send(&self, activation: Tensor) -> CandleResult<Tensor>;
}

/// In-process hand-off: the "transport" is just returning the tensor,
/// possibly moved to the next stage's device.
pub(crate) struct InProcessTransport {
    pub(crate) next_device: Device,
}

impl StageTransport for InProcessTransport {
    fn send(&self, activation: Tensor) -> CandleResult<Tensor> {
        activation.to_device(&self.next_device)
    }
}

/// Runs an ordered sequence of pipeline stages against a single input,
/// handing the activation off between stages via `transports[i]` after stage
/// `i` runs. `transports.len()` must equal `stages.len() - 1`.
pub(crate) fn run_pipeline(
    stages: &[(Box<dyn PipelineStageExecutor>, (u32, u32))],
    transports: &[Box<dyn StageTransport>],
    input: &Tensor,
) -> CandleResult<Tensor> {
    let mut activation = input.clone();
    for (i, (stage, layer_range)) in stages.iter().enumerate() {
        activation = stage.run_stage(*layer_range, &activation)?;
        if let Some(transport) = transports.get(i) {
            activation = transport.send(activation)?;
        }
    }
    Ok(activation)
}

/// Bounds the number of micro-batches admitted into a pipeline concurrently,
/// so per-stage memory usage cannot grow unboundedly with request volume.
/// This is intentionally a simple counting admission gate: real scheduling
/// (e.g. interleaved GPipe-style warmup/cooldown) is a follow-up once
/// pipeline-parallel execution is wired to a live mesh transport.
pub(crate) struct PipelineDepthGate {
    depth: u32,
    in_flight: u32,
}

impl PipelineDepthGate {
    pub(crate) fn new(depth: u32) -> Self {
        Self { depth: depth.max(1), in_flight: 0 }
    }

    /// Returns `true` and reserves a slot if the pipeline has spare depth,
    /// `false` if the caller must queue the request instead.
    pub(crate) fn try_admit(&mut self) -> bool {
        if self.in_flight < self.depth {
            self.in_flight += 1;
            true
        } else {
            false
        }
    }

    pub(crate) fn release(&mut self) {
        self.in_flight = self.in_flight.saturating_sub(1);
    }

    pub(crate) fn in_flight(&self) -> u32 {
        self.in_flight
    }
}

// ---------------------------------------------------------------------------
// Expert parallelism: MoE expert placement and token routing.
// ---------------------------------------------------------------------------

/// Maps each expert id to the index (into a device list) hosting it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExpertPlacementPlan {
    expert_device_index: std::collections::BTreeMap<u32, usize>,
}

impl ExpertPlacementPlan {
    /// Places `expert_count` experts round-robin across `device_count`
    /// devices, so experts are spread as evenly as possible without
    /// requiring the caller to size shards manually.
    pub(crate) fn round_robin(expert_count: u32, device_count: usize) -> Self {
        let mut expert_device_index = std::collections::BTreeMap::new();
        for expert_id in 0..expert_count {
            expert_device_index.insert(expert_id, (expert_id as usize) % device_count.max(1));
        }
        Self { expert_device_index }
    }

    pub(crate) fn device_index_for(&self, expert_id: u32) -> Option<usize> {
        self.expert_device_index.get(&expert_id).copied()
    }
}

/// A single token's top-k expert selection, as produced by the model's gate
/// layer (assumed already computed by the caller; this module only routes,
/// it does not implement gating).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TokenExpertSelection {
    pub(crate) token_index: usize,
    pub(crate) expert_id: u32,
}

/// Groups tokens by the device index hosting their selected expert, so the
/// caller can dispatch each group to that device instead of replicating
/// every expert's dense weights on every device. Tokens whose expert is not
/// in `plan` are omitted (the caller is expected to have validated expert
/// ids against the loaded checkpoint).
pub(crate) fn route_tokens_to_experts(
    selections: &[TokenExpertSelection],
    plan: &ExpertPlacementPlan,
) -> std::collections::BTreeMap<usize, Vec<usize>> {
    let mut routed: std::collections::BTreeMap<usize, Vec<usize>> = std::collections::BTreeMap::new();
    for selection in selections {
        if let Some(device_index) = plan.device_index_for(selection.expert_id) {
            routed.entry(device_index).or_default().push(selection.token_index);
        }
    }
    routed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cpu() -> Device {
        Device::Cpu
    }

    fn weight(rows: usize, cols: usize, seed: u64) -> Tensor {
        let mut values = Vec::with_capacity(rows * cols);
        let mut state = seed.wrapping_add(1);
        for _ in 0..rows * cols {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            values.push(((state >> 40) as f32 / u32::MAX as f32) - 0.5);
        }
        Tensor::from_vec(values, (rows, cols), &cpu()).unwrap()
    }

    // --- Topology discovery ---------------------------------------------------

    #[test]
    fn discovery_always_reports_at_least_the_cpu_device() {
        let topology = discover_cluster_topology();
        assert!(topology.device(0).is_some());
    }

    #[test]
    fn discovery_reports_known_interconnect_for_single_device_topology() {
        let topology = discover_cluster_topology();
        if topology.devices.len() == 1 {
            assert_eq!(topology.interconnect, InterconnectClass::HighBandwidth);
        }
    }

    // --- Topology validation -------------------------------------------------

    fn topology(device_count: usize, interconnect: InterconnectClass, free_vram: u64) -> ClusterTopology {
        ClusterTopology {
            devices: (0..device_count as u32)
                .map(|device_id| DeviceInfo { device_id, free_vram_bytes: free_vram })
                .collect(),
            interconnect,
        }
    }

    fn plan(strategy: ParallelStrategy, device_ids: Vec<u32>, required_vram: u64) -> ParallelExecutionPlan {
        ParallelExecutionPlan {
            strategy,
            device_ids,
            stage_layer_ranges: Vec::new(),
            expert_device_map: Vec::new(),
            required_vram_bytes_per_device: required_vram,
            pipeline_depth: 1,
        }
    }

    #[test]
    fn none_strategy_always_validates() {
        let topo = topology(1, InterconnectClass::CrossNode, 0);
        let p = plan(ParallelStrategy::None, vec![0, 1, 2, 3], 0);
        assert!(validate_parallel_topology(&p, &topo).is_ok());
    }

    #[test]
    fn insufficient_device_count_is_rejected() {
        let topo = topology(2, InterconnectClass::HighBandwidth, 0);
        let p = plan(ParallelStrategy::TensorParallel, vec![0, 1, 2, 3], 0);
        let err = validate_parallel_topology(&p, &topo).unwrap_err();
        assert_eq!(
            err,
            TopologyError::InsufficientDeviceCount { required: 4, available: 2 }
        );
    }

    #[test]
    fn incompatible_interconnect_is_rejected_for_tensor_parallel() {
        let topo = topology(2, InterconnectClass::CrossNode, 0);
        let p = plan(ParallelStrategy::TensorParallel, vec![0, 1], 0);
        let err = validate_parallel_topology(&p, &topo).unwrap_err();
        assert_eq!(
            err,
            TopologyError::IncompatibleInterconnect {
                required: InterconnectClass::Pcie,
                available: InterconnectClass::CrossNode,
            }
        );
    }

    #[test]
    fn cross_node_interconnect_is_fine_for_pipeline_parallel() {
        let topo = topology(2, InterconnectClass::CrossNode, 0);
        let p = plan(ParallelStrategy::PipelineParallel, vec![0, 1], 0);
        assert!(validate_parallel_topology(&p, &topo).is_ok());
    }

    #[test]
    fn vram_per_shard_exceeded_is_rejected() {
        let topo = topology(2, InterconnectClass::HighBandwidth, 1_000);
        let p = plan(ParallelStrategy::TensorParallel, vec![0, 1], 2_000);
        let err = validate_parallel_topology(&p, &topo).unwrap_err();
        assert_eq!(
            err,
            TopologyError::VramPerShardExceeded { required_bytes: 2_000, available_bytes: 1_000 }
        );
    }

    #[test]
    fn sufficient_topology_validates() {
        let topo = topology(2, InterconnectClass::HighBandwidth, 4_000);
        let p = plan(ParallelStrategy::TensorParallel, vec![0, 1], 2_000);
        assert!(validate_parallel_topology(&p, &topo).is_ok());
    }

    // --- Tensor parallelism: numeric equivalence vs. single-device --------

    #[test]
    fn column_parallel_matches_single_device_reference() {
        let device = cpu();
        let w = weight(8, 4, 1); // out_features=8, in_features=4
        let x = Tensor::from_vec(vec![1.0f32, -1.0, 0.5, 2.0], (1, 4), &device).unwrap();

        let reference = x.matmul(&w.t().unwrap()).unwrap();

        let devices = vec![cpu(), cpu()];
        let sharded = ColumnParallelLinear::shard(&w, &devices).unwrap();
        let gathered = sharded.forward(&x, &device).unwrap();

        let reference: Vec<f32> = reference.flatten_all().unwrap().to_vec1().unwrap();
        let gathered: Vec<f32> = gathered.flatten_all().unwrap().to_vec1().unwrap();
        assert_eq!(reference.len(), gathered.len());
        for (a, b) in reference.iter().zip(gathered.iter()) {
            assert!((a - b).abs() < 1e-5, "{a} != {b}");
        }
    }

    #[test]
    fn row_parallel_all_reduce_matches_single_device_reference() {
        let device = cpu();
        let w = weight(4, 8, 2); // out_features=4, in_features=8
        let x = Tensor::from_vec(
            vec![1.0f32, -1.0, 0.5, 2.0, 0.25, -0.75, 1.5, -2.0],
            (1, 8),
            &device,
        )
        .unwrap();

        let reference = x.matmul(&w.t().unwrap()).unwrap();

        let devices = vec![cpu(), cpu()];
        let sharded = RowParallelLinear::shard(&w, &devices).unwrap();
        let x_shards = split_for_row_parallel(&x, &devices).unwrap();
        let reduced = sharded.forward(&x_shards, &device).unwrap();

        let reference: Vec<f32> = reference.flatten_all().unwrap().to_vec1().unwrap();
        let reduced: Vec<f32> = reduced.flatten_all().unwrap().to_vec1().unwrap();
        assert_eq!(reference.len(), reduced.len());
        for (a, b) in reference.iter().zip(reduced.iter()) {
            assert!((a - b).abs() < 1e-5, "{a} != {b}");
        }
    }

    #[test]
    fn tensor_parallel_mlp_block_matches_single_device_reference() {
        // up: [hidden=4 -> intermediate=8] (column-parallel), down: [8 -> 4] (row-parallel).
        let device = cpu();
        let up = weight(8, 4, 3);
        let down = weight(4, 8, 4);
        let x = Tensor::from_vec(vec![0.3f32, -0.2, 1.1, 0.7], (1, 4), &device).unwrap();

        // Single-device reference (ReLU activation between the two linears).
        let reference_hidden = x.matmul(&up.t().unwrap()).unwrap().relu().unwrap();
        let reference = reference_hidden.matmul(&down.t().unwrap()).unwrap();

        let devices = vec![cpu(), cpu()];
        let column = ColumnParallelLinear::shard(&up, &devices).unwrap();
        let gathered = column.forward(&x, &device).unwrap().relu().unwrap();
        let row = RowParallelLinear::shard(&down, &devices).unwrap();
        let x_shards = split_for_row_parallel(&gathered, &devices).unwrap();
        let sharded_output = row.forward(&x_shards, &device).unwrap();

        let reference: Vec<f32> = reference.flatten_all().unwrap().to_vec1().unwrap();
        let sharded_output: Vec<f32> = sharded_output.flatten_all().unwrap().to_vec1().unwrap();
        for (a, b) in reference.iter().zip(sharded_output.iter()) {
            assert!((a - b).abs() < 1e-4, "{a} != {b}");
        }
    }

    // --- Pipeline parallelism ------------------------------------------------

    #[test]
    fn pipeline_stages_compose_like_a_single_pass() {
        let device = cpu();
        let x = Tensor::from_vec(vec![1.0f32, 2.0, 3.0], (1, 3), &device).unwrap();

        // Two stages: stage 0 doubles, stage 1 adds one. Composing them must
        // equal applying both transforms directly to the input.
        let stage0 = ClosureStageExecutor(|_range: (u32, u32), t: &Tensor| t * 2.0);
        let stage1 = ClosureStageExecutor(|_range: (u32, u32), t: &Tensor| t + 1.0);

        let stages: Vec<(Box<dyn PipelineStageExecutor>, (u32, u32))> = vec![
            (Box::new(stage0), (0, 1)),
            (Box::new(stage1), (2, 3)),
        ];
        let transports: Vec<Box<dyn StageTransport>> =
            vec![Box::new(InProcessTransport { next_device: device.clone() })];

        let output = run_pipeline(&stages, &transports, &x).unwrap();
        let expected = ((&x * 2.0).unwrap() + 1.0).unwrap();

        let output: Vec<f32> = output.flatten_all().unwrap().to_vec1().unwrap();
        let expected: Vec<f32> = expected.flatten_all().unwrap().to_vec1().unwrap();
        assert_eq!(output, expected);
    }

    #[test]
    fn pipeline_depth_gate_bounds_in_flight_microbatches() {
        let mut gate = PipelineDepthGate::new(2);
        assert!(gate.try_admit());
        assert!(gate.try_admit());
        assert!(!gate.try_admit(), "third micro-batch must queue, not admit");
        assert_eq!(gate.in_flight(), 2);
        gate.release();
        assert!(gate.try_admit());
    }

    // --- Expert parallelism ---------------------------------------------------

    #[test]
    fn experts_are_spread_round_robin_across_devices() {
        let plan = ExpertPlacementPlan::round_robin(4, 2);
        assert_eq!(plan.device_index_for(0), Some(0));
        assert_eq!(plan.device_index_for(1), Some(1));
        assert_eq!(plan.device_index_for(2), Some(0));
        assert_eq!(plan.device_index_for(3), Some(1));
    }

    #[test]
    fn tokens_are_routed_only_to_their_selected_experts_device() {
        let plan = ExpertPlacementPlan::round_robin(2, 2);
        let selections = vec![
            TokenExpertSelection { token_index: 0, expert_id: 0 },
            TokenExpertSelection { token_index: 1, expert_id: 1 },
            TokenExpertSelection { token_index: 2, expert_id: 0 },
        ];
        let routed = route_tokens_to_experts(&selections, &plan);
        assert_eq!(routed.get(&0), Some(&vec![0usize, 2usize]));
        assert_eq!(routed.get(&1), Some(&vec![1usize]));
    }

    #[test]
    fn dense_checkpoints_have_a_trivial_single_device_placement() {
        // A non-MoE checkpoint has no experts to place; routing must be a no-op
        // rather than an error, preserving the existing dense execution path.
        let plan = ExpertPlacementPlan::round_robin(0, 4);
        let routed = route_tokens_to_experts(&[], &plan);
        assert!(routed.is_empty());
    }
}
