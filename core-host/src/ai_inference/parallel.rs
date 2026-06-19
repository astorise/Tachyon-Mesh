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

use candle_core::{Device, Result as CandleResult, Tensor};

/// Pure validation types/logic (no GPU runtime dependency) live in the
/// `parallel-topology` crate so they can be shared with
/// `system-faas-config-api`, which validates a plan's *shape* at
/// `apply-model-deployment` time before any hardware topology is known.
// Re-exported for the dispatch path that wires a validated plan into the
// real tensor/pipeline/expert execution below (tracked separately; not yet
// wired), hence `allow(unused_imports)` until that caller lands.
#[allow(unused_imports)]
pub(crate) use parallel_topology::{
    validate_parallel_topology, ClusterTopology, DeviceInfo, InterconnectClass,
    ParallelExecutionPlan, ParallelStrategy, TopologyError,
};

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

/// Drives `micro_batches` through `stages`/`transports` with a GPipe-style
/// schedule: at each tick, every microbatch currently admitted is advanced by
/// exactly one stage, and new microbatches are admitted as soon as
/// `depth_gate` has spare capacity. This is the schedule that lets stage `i`
/// work on microbatch `k` while stage `i-1` works on microbatch `k+1` once
/// each stage runs on its own device/thread; this reference scheduler still
/// executes every tick in-process and sequentially, so it proves the
/// admission/ordering logic without yet producing real wall-clock overlap —
/// that requires one execution thread per stage, left as follow-up.
pub(crate) fn run_pipeline_microbatched(
    stages: &[(Box<dyn PipelineStageExecutor>, (u32, u32))],
    transports: &[Box<dyn StageTransport>],
    micro_batches: Vec<Tensor>,
    depth_gate: &mut PipelineDepthGate,
) -> CandleResult<Vec<Tensor>> {
    let num_stages = stages.len();
    let total = micro_batches.len();
    let mut cursor = vec![0usize; total];
    let mut activation: Vec<Option<Tensor>> = micro_batches.into_iter().map(Some).collect();
    let mut outputs: Vec<Option<Tensor>> = vec![None; total];
    let mut admitted = vec![false; total];
    let mut next_to_admit = 0usize;
    let mut finished = 0usize;

    while finished < total {
        while next_to_admit < total && depth_gate.try_admit() {
            admitted[next_to_admit] = true;
            next_to_admit += 1;
        }

        for k in 0..total {
            if !admitted[k] || outputs[k].is_some() {
                continue;
            }
            let stage_idx = cursor[k];
            let input = activation[k]
                .take()
                .expect("an admitted, unfinished microbatch always has a pending activation");
            let (stage, layer_range) = &stages[stage_idx];
            let mut next = stage.run_stage(*layer_range, &input)?;
            if let Some(transport) = transports.get(stage_idx) {
                next = transport.send(next)?;
            }
            if stage_idx + 1 == num_stages {
                outputs[k] = Some(next);
                depth_gate.release();
                finished += 1;
            } else {
                activation[k] = Some(next);
                cursor[k] = stage_idx + 1;
            }
        }
    }

    Ok(outputs
        .into_iter()
        .map(|o| o.expect("every microbatch is finished by the time the loop above exits"))
        .collect())
}

// ---------------------------------------------------------------------------
// Cross-node activation transport over a plain TCP socket.
//
// `design.md` for this change describes reusing "the existing mesh transport
// (gRPC/HTTP2 capability `grpc-http2`)" for this hand-off. That turned out to
// be inaccurate: `grpc-http2` (see `openspec/specs/grpc-http2/spec.md`) is the
// FaaS guest-request HTTP/gRPC router, not a node-to-node transport — there
// was no existing mesh transport to reuse. `TcpStageTransport` below is a new,
// minimal, genuinely networked transport built for this purpose: it performs
// a real OS-socket round trip and satisfies the `StageTransport` contract
// (push an activation, get back the tensor the peer produced), so pointing
// `addr` at a remote host requires no further code changes to run cross-node.
// Real gRPC/HTTP2 framing can replace the wire format later without changing
// `StageTransport`'s contract or any caller.
// ---------------------------------------------------------------------------

fn io_err(err: std::io::Error) -> candle_core::Error {
    candle_core::Error::Msg(format!("pipeline-stage TCP transport I/O error: {err}"))
}

/// Encodes a tensor as `[ndims:u32][dims:u32 * ndims][f32 data, little-endian]`.
/// Activations are always moved through CPU memory for transport; each end
/// handles its own device placement.
fn encode_tensor(tensor: &Tensor) -> CandleResult<Vec<u8>> {
    let dims = tensor.dims().to_vec();
    let data = tensor.flatten_all()?.to_dtype(candle_core::DType::F32)?.to_vec1::<f32>()?;
    let mut bytes = Vec::with_capacity(4 + dims.len() * 4 + data.len() * 4);
    bytes.extend_from_slice(&(dims.len() as u32).to_le_bytes());
    for dim in &dims {
        bytes.extend_from_slice(&(*dim as u32).to_le_bytes());
    }
    for value in &data {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    Ok(bytes)
}

/// Inverse of [`encode_tensor`]; rebuilds the tensor directly on `device`.
fn decode_tensor(bytes: &[u8], device: &Device) -> CandleResult<Tensor> {
    let read_u32 = |offset: usize| -> CandleResult<u32> {
        bytes
            .get(offset..offset + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .ok_or_else(|| candle_core::Error::Msg("truncated tensor frame: missing header".into()))
    };
    let ndims = read_u32(0)? as usize;
    let mut dims = Vec::with_capacity(ndims);
    let mut offset = 4;
    for _ in 0..ndims {
        dims.push(read_u32(offset)? as usize);
        offset += 4;
    }
    let elem_count: usize = dims.iter().product();
    let data_bytes = bytes
        .get(offset..offset + elem_count * 4)
        .ok_or_else(|| candle_core::Error::Msg("truncated tensor frame: missing payload".into()))?;
    let data: Vec<f32> = data_bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    Tensor::from_vec(data, dims, device)
}

fn write_frame(stream: &mut std::net::TcpStream, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    stream.write_all(&(bytes.len() as u32).to_le_bytes())?;
    stream.write_all(bytes)?;
    stream.flush()
}

fn read_frame(stream: &mut std::net::TcpStream) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes)?;
    let len = u32::from_le_bytes(len_bytes) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

/// Client-side cross-node [`StageTransport`]: connects to `addr` (the node
/// hosting the next pipeline stage), pushes the activation, and blocks for
/// the response tensor the peer computed.
pub(crate) struct TcpStageTransport {
    addr: std::net::SocketAddr,
}

impl TcpStageTransport {
    pub(crate) fn new(addr: std::net::SocketAddr) -> Self {
        Self { addr }
    }

    /// Peer-side counterpart: accepts exactly one activation frame on
    /// `listener`, runs `handle` (typically the next stage's
    /// `PipelineStageExecutor::run_stage`) on the decoded tensor, and writes
    /// the result back on the same connection.
    pub(crate) fn serve_one(
        listener: &std::net::TcpListener,
        device: &Device,
        handle: impl FnOnce(Tensor) -> CandleResult<Tensor>,
    ) -> CandleResult<()> {
        let (mut stream, _) = listener.accept().map_err(io_err)?;
        let request = read_frame(&mut stream).map_err(io_err)?;
        let input = decode_tensor(&request, device)?;
        let output = handle(input)?;
        let response = encode_tensor(&output)?;
        write_frame(&mut stream, &response).map_err(io_err)
    }
}

impl StageTransport for TcpStageTransport {
    fn send(&self, activation: Tensor) -> CandleResult<Tensor> {
        let device = activation.device().clone();
        let bytes = encode_tensor(&activation)?;
        let mut stream = std::net::TcpStream::connect(self.addr).map_err(io_err)?;
        write_frame(&mut stream, &bytes).map_err(io_err)?;
        let response = read_frame(&mut stream).map_err(io_err)?;
        decode_tensor(&response, &device)
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

    // Topology validation itself (insufficient device count, incompatible
    // interconnect, VRAM-per-shard exceeded) is tested in the
    // `parallel-topology` crate, which owns `validate_parallel_topology`.

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

    #[test]
    fn microbatched_pipeline_matches_running_each_microbatch_through_run_pipeline_alone() {
        let device = cpu();
        let stage0 = ClosureStageExecutor(|_range: (u32, u32), t: &Tensor| t * 2.0);
        let stage1 = ClosureStageExecutor(|_range: (u32, u32), t: &Tensor| t + 1.0);
        let stages: Vec<(Box<dyn PipelineStageExecutor>, (u32, u32))> = vec![
            (Box::new(stage0), (0, 1)),
            (Box::new(stage1), (2, 3)),
        ];
        let transports: Vec<Box<dyn StageTransport>> =
            vec![Box::new(InProcessTransport { next_device: device.clone() })];

        let micro_batches: Vec<Tensor> = (0..5)
            .map(|i| Tensor::from_vec(vec![i as f32], (1, 1), &device).unwrap())
            .collect();
        let expected: Vec<f32> = micro_batches
            .iter()
            .map(|x| ((x * 2.0).unwrap() + 1.0).unwrap().to_vec2::<f32>().unwrap()[0][0])
            .collect();

        // Depth 2: never more than 2 microbatches in flight, but every
        // microbatch must still reach the same output as the single-batch
        // `run_pipeline` reference.
        let mut gate = PipelineDepthGate::new(2);
        let outputs =
            run_pipeline_microbatched(&stages, &transports, micro_batches, &mut gate).unwrap();
        assert_eq!(gate.in_flight(), 0, "every microbatch must release its slot on completion");

        let outputs: Vec<f32> =
            outputs.iter().map(|t| t.to_vec2::<f32>().unwrap()[0][0]).collect();
        assert_eq!(outputs, expected);
    }

    #[test]
    fn tcp_stage_transport_round_trips_an_activation_over_a_real_socket() {
        let device = cpu();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server = std::thread::spawn(move || {
            TcpStageTransport::serve_one(&listener, &Device::Cpu, |t| t + 1.0).unwrap();
        });

        let transport = TcpStageTransport::new(addr);
        let activation = Tensor::from_vec(vec![1.0f32, 2.0, 3.0], (1, 3), &device).unwrap();
        let response = transport.send(activation).unwrap();
        server.join().unwrap();

        let response: Vec<f32> = response.flatten_all().unwrap().to_vec1().unwrap();
        assert_eq!(response, vec![2.0, 3.0, 4.0]);
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
