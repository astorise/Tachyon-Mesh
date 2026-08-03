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

#[cfg(feature = "candle-cuda")]
use candle_core::{op::BackpropOp, CudaStorage, Storage};
use candle_core::{DType, Device, IndexOp, Module, Result as CandleResult, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::with_tracing::{linear_no_bias as linear, Linear};
#[cfg(all(feature = "candle-cuda", target_os = "linux"))]
use std::os::raw::c_int;
#[cfg(feature = "candle-cuda")]
use std::sync::OnceLock;

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
    #[cfg_attr(not(feature = "candle-cuda"), allow(unused_mut))]
    let mut devices = vec![DeviceInfo {
        device_id: 0,
        free_vram_bytes: 0,
    }];

    // With the `candle-cuda` Cargo feature compiled in (a separate, sibling
    // feature to `nvfp4-cuda` — enabling `nvfp4-cuda` alone does NOT pull in
    // `candle-cuda`; see core-host/Cargo.toml), `cuda_if_available` actually
    // opens devices, so this loop enumerates every real GPU ordinal.
    // `free_vram_bytes` stays `0` (unknown) until NVML telemetry lands;
    // `validate_parallel_topology` only enforces the VRAM check when the plan
    // declares a non-zero per-shard requirement, so a `0` here never causes a
    // spurious rejection.
    #[cfg(feature = "candle-cuda")]
    {
        let mut ordinal = 0u32;
        loop {
            match Device::cuda_if_available(ordinal as usize) {
                Ok(Device::Cuda(_)) => {
                    devices.push(DeviceInfo {
                        device_id: ordinal + 1,
                        free_vram_bytes: free_vram_bytes(ordinal),
                    });
                    ordinal += 1;
                }
                _ => break,
            }
            if ordinal > 64 {
                break;
            }
        }
    }

    let interconnect = if devices.len() > 1 {
        InterconnectClass::Pcie
    } else {
        InterconnectClass::HighBandwidth
    };

    ClusterTopology {
        devices,
        interconnect,
    }
}

/// Reports `ordinal`'s free VRAM via NVML (`nvmlDeviceGetMemoryInfo`).
/// `nvml-wrapper` dlopens `libnvidia-ml.so` at runtime rather than linking
/// against it at build time, so `Nvml::init()` failing (no driver, no
/// permissions, non-NVIDIA host) degrades to `0` ("unknown") exactly like
/// the pre-NVML behavior — it never panics and never fails the build.
/// `validate_parallel_topology` only enforces the VRAM check when a plan
/// declares a non-zero per-shard requirement, so a `0` here never causes a
/// spurious rejection.
#[cfg(feature = "candle-cuda")]
fn free_vram_bytes(ordinal: u32) -> u64 {
    static NVML: OnceLock<Option<nvml_wrapper::Nvml>> = OnceLock::new();
    let Some(nvml) = NVML.get_or_init(|| nvml_wrapper::Nvml::init().ok()) else {
        return 0;
    };
    nvml.device_by_index(ordinal)
        .and_then(|d| d.memory_info())
        .map(|m| m.free)
        .unwrap_or(0)
}

/// One tensor-parallel shard group's NCCL communicators, one per
/// participating CUDA device, created once (NCCL communicator init is
/// expensive: it allocates and exchanges out-of-band rendezvous state) and
/// reused for every `RowParallelLinear::forward` call in that group's
/// lifetime, mirroring how `TensorParallelCache` is built once per model and
/// threaded through layers.
#[cfg(feature = "candle-cuda")]
pub(crate) struct NcclShardGroup {
    comms: Vec<cudarc::nccl::Comm>,
    // `cudarc::nccl::Comm` wraps a raw `ncclComm_t` pointer and is therefore
    // neither `Send` nor `Sync` on its own. NCCL only requires that a given
    // communicator not be driven by multiple threads concurrently, so this
    // lock serializes every collective call through `all_reduce_sum` to make
    // sharing this group behind an `Arc` across worker threads sound.
    lock: std::sync::Mutex<()>,
}

// SAFETY: see the `lock` field above — every access to `comms` is
// serialized through `all_reduce_sum`, which holds `lock` for the duration
// of every NCCL collective call.
#[cfg(feature = "candle-cuda")]
unsafe impl Send for NcclShardGroup {}
#[cfg(feature = "candle-cuda")]
unsafe impl Sync for NcclShardGroup {}

#[cfg(feature = "candle-cuda")]
impl NcclShardGroup {
    /// Builds one communicator per device. Returns `None` (rather than an
    /// error) if fewer than 2 devices are given, any device is not
    /// `Device::Cuda`, or communicator creation fails, so callers can
    /// transparently fall back to the host-staged sum.
    ///
    /// Real deployments (`TensorParallelBlock`/`TensorParallelLlama`) shard
    /// across distinct physical GPUs, so the common path uses
    /// `ncclCommInitAll`'s single-process, multi-device API
    /// (`Comm::from_devices`), which requires every device ordinal to be
    /// distinct. The per-rank fallback (`Comm::from_rank`/`ncclCommInitRank`)
    /// below is kept for callers that pass duplicate ordinals, but modern
    /// NCCL rejects multiple ranks sharing one GPU even through that lower-
    /// level API (confirmed on real hardware: `ncclGroupEnd` surfaces a
    /// deferred error for it), so it is not expected to succeed in practice
    /// — `nccl_all_reduce_matches_cpu_staged_reference` exercises the real
    /// `Comm::from_devices` path with 2 distinct GPUs instead.
    pub(crate) fn try_new(devices: &[Device]) -> Option<Self> {
        let mut streams = Vec::with_capacity(devices.len());
        for device in devices {
            match device {
                Device::Cuda(cuda_device) => streams.push(cuda_device.cuda_stream()),
                _ => return None,
            }
        }
        if streams.len() < 2 {
            return None;
        }

        let ordinals: Vec<usize> = streams.iter().map(|s| s.context().ordinal()).collect();
        let all_distinct = {
            let mut sorted = ordinals.clone();
            sorted.sort_unstable();
            sorted.dedup();
            sorted.len() == ordinals.len()
        };

        let comms = if all_distinct {
            match cudarc::nccl::Comm::from_devices(streams) {
                Ok(comms) => comms,
                Err(err) => {
                    eprintln!("NcclShardGroup: Comm::from_devices failed: {:?}", err.0);
                    return None;
                }
            }
        } else {
            let world_size = streams.len();
            let id = match cudarc::nccl::Id::new() {
                Ok(id) => id,
                Err(err) => {
                    eprintln!("NcclShardGroup: Id::new failed: {:?}", err.0);
                    return None;
                }
            };
            // Per-rank `ncclCommInitRank` calls driven from one thread are
            // subject to the same group requirement as the collective calls
            // in `all_reduce_sum`: every rank's init must be posted before
            // any of them blocks waiting on its peers.
            if let Err(err) = cudarc::nccl::group_start() {
                eprintln!("NcclShardGroup: group_start failed: {:?}", err.0);
                return None;
            }
            let mut comms = Vec::with_capacity(world_size);
            for (rank, stream) in streams.into_iter().enumerate() {
                match cudarc::nccl::Comm::from_rank(stream, rank, world_size, id) {
                    Ok(comm) => comms.push(comm),
                    Err(err) => {
                        eprintln!(
                            "NcclShardGroup: Comm::from_rank(rank={rank}, world_size={world_size}) failed: {:?}",
                            err.0
                        );
                        break;
                    }
                }
            }
            if let Err(err) = cudarc::nccl::group_end() {
                eprintln!(
                    "NcclShardGroup: group_end failed (deferred error from per-rank init): {:?}",
                    err.0
                );
                return None;
            }
            if comms.len() != world_size {
                eprintln!(
                    "NcclShardGroup: only {}/{world_size} ranks initialized",
                    comms.len()
                );
                return None;
            }
            comms
        };

        Some(Self {
            comms,
            lock: std::sync::Mutex::new(()),
        })
    }

    /// Builds communicators with `ncclCommInitRank` after exchanging one
    /// NCCL unique id over TCP. This is the multi-process/multi-node path:
    /// rank 0 binds `bootstrap.master_addr`, sends the generated id to each
    /// remote process, and every process initializes its local CUDA devices
    /// as a contiguous global rank range starting at
    /// `bootstrap.rank_offset`.
    ///
    /// The caller owns orchestration: every process must use the same
    /// `world_size`, non-overlapping rank ranges, and the master must set
    /// `peer_count` to the number of remote processes that will connect.
    /// Returning `None` preserves the existing host-staged fallback behavior
    /// for callers that attach the group opportunistically.
    pub(crate) fn try_new_networked(
        devices: &[Device],
        bootstrap: &NcclTcpBootstrap,
    ) -> Option<Self> {
        let mut streams = Vec::with_capacity(devices.len());
        for device in devices {
            match device {
                Device::Cuda(cuda_device) => streams.push(cuda_device.cuda_stream()),
                _ => return None,
            }
        }
        if streams.is_empty()
            || bootstrap.world_size < 2
            || bootstrap.rank_offset + streams.len() > bootstrap.world_size
        {
            return None;
        }

        let id = match bootstrap.obtain_id() {
            Ok(id) => id,
            Err(err) => {
                eprintln!("NcclShardGroup: TCP NCCL bootstrap failed: {err}");
                return None;
            }
        };

        if let Err(err) = cudarc::nccl::group_start() {
            eprintln!("NcclShardGroup: networked group_start failed: {:?}", err.0);
            return None;
        }
        let mut comms = Vec::with_capacity(streams.len());
        for (local_rank, stream) in streams.into_iter().enumerate() {
            let rank = bootstrap.rank_offset + local_rank;
            match cudarc::nccl::Comm::from_rank(stream, rank, bootstrap.world_size, id) {
                Ok(comm) => comms.push(comm),
                Err(err) => {
                    eprintln!(
                        "NcclShardGroup: networked Comm::from_rank(rank={rank}, world_size={}) failed: {:?}",
                        bootstrap.world_size, err.0
                    );
                    break;
                }
            }
        }
        if let Err(err) = cudarc::nccl::group_end() {
            eprintln!(
                "NcclShardGroup: networked group_end failed (deferred init error): {:?}",
                err.0
            );
            return None;
        }
        if comms.len() != devices.len() {
            eprintln!(
                "NcclShardGroup: only {}/{} local networked ranks initialized",
                comms.len(),
                devices.len()
            );
            return None;
        }

        Some(Self {
            comms,
            lock: std::sync::Mutex::new(()),
        })
    }

    /// Sums `partials[i]` (resident on the i-th participating CUDA device,
    /// must be contiguous `DType::F32`) across every communicator via a real
    /// NCCL `AllReduce`, then moves the reduced tensor onto `reduce_device`.
    /// Every device ends up holding the identical sum, so any one of them
    /// (here, the first) is a valid source for that final move.
    fn all_reduce_sum(&self, partials: &[Tensor], reduce_device: &Device) -> CandleResult<Tensor> {
        use cudarc::nccl::ReduceOp;

        let _guard = match self.lock.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        // NCCL requires that when one thread drives multiple communicators
        // for distinct devices (our case: `self.comms` holds one communicator
        // per participating GPU, all launched from this single thread), the
        // per-device calls be enclosed in a group so they are all *posted*
        // before any of them blocks waiting on its peers — otherwise the
        // first iteration's `all_reduce` can hang waiting for ranks whose
        // calls haven't been issued yet. See
        // https://docs.nvidia.com/deeplearning/nccl/user-guide/docs/usage/groups.html
        cudarc::nccl::group_start()
            .map_err(|e| candle_core::Error::Msg(format!("NCCL group_start failed: {e:?}")))?;

        let mut reduced_on_device: Vec<Tensor> = Vec::with_capacity(partials.len());
        let mut first_error: Option<candle_core::Error> = None;
        for (comm, partial) in self.comms.iter().zip(partials.iter()) {
            let step = || -> CandleResult<Tensor> {
                let cuda_device = match partial.device() {
                    Device::Cuda(d) => d,
                    _ => candle_core::bail!(
                        "NCCL all-reduce requires every partial to be on a CUDA device"
                    ),
                };
                let (storage, layout) = partial.storage_and_layout();
                if !layout.is_contiguous() {
                    candle_core::bail!("NCCL all-reduce requires a contiguous partial tensor");
                }
                let src = match &*storage {
                    Storage::Cuda(cs) => cs.as_cuda_slice::<f32>()?,
                    _ => candle_core::bail!("NCCL all-reduce requires CUDA storage"),
                };
                let mut dst = cuda_device.alloc_zeros::<f32>(src.len()).map_err(|e| {
                    candle_core::Error::Msg(format!("NCCL all-reduce alloc failed: {e:?}"))
                })?;
                comm.all_reduce(src, &mut dst, &ReduceOp::Sum)
                    .map_err(|e| {
                        candle_core::Error::Msg(format!("NCCL AllReduce failed: {e:?}"))
                    })?;
                let storage = CudaStorage::wrap_cuda_slice(dst, cuda_device.clone());
                Ok(Tensor::from_storage(
                    Storage::Cuda(storage),
                    partial.shape().clone(),
                    BackpropOp::none(),
                    false,
                ))
            };
            match step() {
                Ok(tensor) => reduced_on_device.push(tensor),
                Err(e) => {
                    first_error = Some(e);
                    break;
                }
            }
        }

        // group_end() must run even on error: it's what actually posts the
        // queued NCCL calls, and every rank's group must close in step or
        // later collectives on these communicators will desync.
        cudarc::nccl::group_end()
            .map_err(|e| candle_core::Error::Msg(format!("NCCL group_end failed: {e:?}")))?;
        if let Some(e) = first_error {
            return Err(e);
        }
        // Every device's stream must finish the collective before the
        // freshly-allocated buffers are read back by the `to_device` move.
        for device in partials.iter().map(Tensor::device) {
            if let Device::Cuda(cuda_device) = device {
                cuda_device.cuda_stream().synchronize().map_err(|e| {
                    candle_core::Error::Msg(format!("CUDA stream sync failed: {e:?}"))
                })?;
            }
        }
        let result = reduced_on_device.into_iter().next().ok_or_else(|| {
            candle_core::Error::Msg("NCCL all-reduce requires at least one shard".into())
        })?;
        result.to_device(reduce_device)
    }
}

const NCCL_UNIQUE_ID_LEN: usize = 128;

#[cfg(feature = "candle-cuda")]
fn nccl_id_to_bytes(id: &cudarc::nccl::Id) -> [u8; NCCL_UNIQUE_ID_LEN] {
    let mut bytes = [0u8; NCCL_UNIQUE_ID_LEN];
    for (dst, src) in bytes.iter_mut().zip(id.internal().iter()) {
        *dst = *src as u8;
    }
    bytes
}

#[cfg(feature = "candle-cuda")]
fn nccl_id_from_bytes(bytes: [u8; NCCL_UNIQUE_ID_LEN]) -> cudarc::nccl::Id {
    let mut internal = [0 as std::os::raw::c_char; NCCL_UNIQUE_ID_LEN];
    for (dst, src) in internal.iter_mut().zip(bytes.iter()) {
        *dst = *src as std::os::raw::c_char;
    }
    cudarc::nccl::Id::uninit(internal)
}

fn broadcast_nccl_rendezvous_bytes(
    listener: &std::net::TcpListener,
    bytes: &[u8; NCCL_UNIQUE_ID_LEN],
    peer_count: usize,
) -> std::io::Result<()> {
    for _ in 0..peer_count {
        let (mut stream, _) = listener.accept()?;
        write_frame(&mut stream, bytes)?;
    }
    Ok(())
}

fn fetch_nccl_rendezvous_bytes(
    master_addr: std::net::SocketAddr,
) -> std::io::Result<[u8; NCCL_UNIQUE_ID_LEN]> {
    let mut stream = std::net::TcpStream::connect(master_addr)?;
    let frame = read_frame(&mut stream)?;
    frame
        .try_into()
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid NCCL id size"))
}

/// TCP rendezvous settings for inter-process NCCL initialization.
#[cfg(feature = "candle-cuda")]
#[derive(Clone, Debug)]
pub(crate) struct NcclTcpBootstrap {
    pub(crate) role: NcclTcpBootstrapRole,
    pub(crate) rank_offset: usize,
    pub(crate) world_size: usize,
}

#[cfg(feature = "candle-cuda")]
#[derive(Clone, Debug)]
pub(crate) enum NcclTcpBootstrapRole {
    Master {
        bind_addr: std::net::SocketAddr,
        peer_count: usize,
    },
    Worker {
        master_addr: std::net::SocketAddr,
    },
}

#[cfg(feature = "candle-cuda")]
impl NcclTcpBootstrap {
    pub(crate) fn master(
        bind_addr: std::net::SocketAddr,
        rank_offset: usize,
        world_size: usize,
        peer_count: usize,
    ) -> Self {
        Self {
            role: NcclTcpBootstrapRole::Master {
                bind_addr,
                peer_count,
            },
            rank_offset,
            world_size,
        }
    }

    pub(crate) fn worker(
        master_addr: std::net::SocketAddr,
        rank_offset: usize,
        world_size: usize,
    ) -> Self {
        Self {
            role: NcclTcpBootstrapRole::Worker { master_addr },
            rank_offset,
            world_size,
        }
    }

    fn obtain_id(&self) -> std::io::Result<cudarc::nccl::Id> {
        match self.role {
            NcclTcpBootstrapRole::Master {
                bind_addr,
                peer_count,
            } => {
                let id =
                    cudarc::nccl::Id::new().map_err(|e| std::io::Error::other(format!("{e:?}")))?;
                let listener = std::net::TcpListener::bind(bind_addr)?;
                let bytes = nccl_id_to_bytes(&id);
                broadcast_nccl_rendezvous_bytes(&listener, &bytes, peer_count)?;
                Ok(id)
            }
            NcclTcpBootstrapRole::Worker { master_addr } => {
                fetch_nccl_rendezvous_bytes(master_addr).map(nccl_id_from_bytes)
            }
        }
    }
}

#[cfg(all(feature = "candle-cuda", target_os = "linux"))]
#[repr(C)]
struct CpuSet {
    bits: [usize; 16],
}

#[cfg(all(feature = "candle-cuda", target_os = "linux"))]
extern "C" {
    fn sched_setaffinity(pid: c_int, cpusetsize: usize, mask: *const CpuSet) -> c_int;
}

#[cfg(all(feature = "candle-cuda", target_os = "linux"))]
fn parse_linux_cpu_list(list: &str) -> CandleResult<Vec<usize>> {
    let mut cpus = Vec::new();
    for part in list.trim().split(',').filter(|p| !p.is_empty()) {
        if let Some((start, end)) = part.split_once('-') {
            let start: usize = start.parse().map_err(|_| {
                candle_core::Error::Msg(format!("invalid NUMA CPU list entry: {part}"))
            })?;
            let end: usize = end.parse().map_err(|_| {
                candle_core::Error::Msg(format!("invalid NUMA CPU list entry: {part}"))
            })?;
            if start > end {
                candle_core::bail!("invalid NUMA CPU range: {part}");
            }
            cpus.extend(start..=end);
        } else {
            cpus.push(part.parse().map_err(|_| {
                candle_core::Error::Msg(format!("invalid NUMA CPU list entry: {part}"))
            })?);
        }
    }
    Ok(cpus)
}

/// Pins the current process to CPUs local to `node_id` using Linux sysfs'
/// `/sys/devices/system/node/nodeN/cpulist`. Call this before initializing
/// CUDA/NCCL workers for a node-local rank group so host-side NCCL progress
/// threads and staging work stay close to the target NUMA domain.
#[cfg(all(feature = "candle-cuda", target_os = "linux"))]
pub(crate) fn bind_current_process_to_numa_node(node_id: u32) -> CandleResult<()> {
    let path = format!("/sys/devices/system/node/node{node_id}/cpulist");
    let cpulist = std::fs::read_to_string(&path).map_err(|e| {
        candle_core::Error::Msg(format!("failed to read NUMA CPU list {path}: {e}"))
    })?;
    let cpus = parse_linux_cpu_list(&cpulist)?;
    if cpus.is_empty() {
        candle_core::bail!("NUMA node {node_id} has an empty CPU list");
    }

    let mut set = CpuSet { bits: [0; 16] };
    for cpu in cpus {
        let word = cpu / usize::BITS as usize;
        let bit = cpu % usize::BITS as usize;
        if word < set.bits.len() {
            set.bits[word] |= 1usize << bit;
        }
    }

    let rc = unsafe { sched_setaffinity(0, std::mem::size_of::<CpuSet>(), &set) };
    if rc == 0 {
        Ok(())
    } else {
        Err(candle_core::Error::Msg(format!(
            "sched_setaffinity failed for NUMA node {node_id}: {}",
            std::io::Error::last_os_error()
        )))
    }
}

// ---------------------------------------------------------------------------
// Tensor parallelism: Megatron-style column/row-parallel linear layers.
// ---------------------------------------------------------------------------

/// The Megatron-style sharded linears, from `candle-nn` (candle #3828).
///
/// They were written here first, for want of anywhere else to put them, and
/// they were never mesh-specific: two `Tensor` operations and a device list.
/// What is mesh-specific is which collective performs the row-parallel
/// reduction, and upstream leaves exactly that to the caller — see
/// [`NcclReducer`].
pub(crate) use candle_nn::{
    split_for_row_parallel, ColumnParallelLinear, RowParallelLinear, SumReducer, TensorReducer,
};

/// The reduction strategy a [`RowParallelLinear`] uses, bound to this host's
/// communicator.
///
/// A row-parallel layer's all-reduce is the whole communication cost of tensor
/// parallelism, and how it travels is a deployment decision rather than a
/// modelling one: NCCL between GPUs on a node, and a host-staged sum
/// everywhere else. This is the seam candle asks for, holding the one thing
/// upstream cannot know.
pub(crate) struct NcclReducer {
    /// Real NCCL communicators for this shard group, when one was built.
    /// `None` — and every non-CUDA build — falls back to the host-staged sum,
    /// which is `SumReducer` and is what this code did before NCCL existed
    /// here.
    #[cfg(feature = "candle-cuda")]
    group: Option<std::sync::Arc<NcclShardGroup>>,
}

impl NcclReducer {
    /// One communicator group per tensor-parallel shard group, shared across
    /// every layer that reduces through it.
    #[cfg(feature = "candle-cuda")]
    pub(crate) fn new(group: Option<std::sync::Arc<NcclShardGroup>>) -> Self {
        Self { group }
    }

    #[cfg(not(feature = "candle-cuda"))]
    pub(crate) fn new() -> Self {
        Self {}
    }
}

impl TensorReducer for NcclReducer {
    /// Real NCCL `AllReduce` when a communicator group is attached and every
    /// partial is a CUDA `DType::F32` tensor with more than one device
    /// participating; the host-staged sum otherwise (no `candle-cuda`, single
    /// device, `Device::Cpu`, or a dtype not wired into the NCCL path).
    fn all_reduce(&self, shards: &[Tensor], reduce_device: &Device) -> CandleResult<Tensor> {
        #[cfg(feature = "candle-cuda")]
        if let Some(group) = &self.group {
            let eligible = shards.len() > 1
                && shards
                    .iter()
                    .all(|t| matches!(t.device(), Device::Cuda(_)) && t.dtype() == DType::F32);
            if eligible {
                return group.all_reduce_sum(shards, reduce_device);
            }
        }
        SumReducer.all_reduce(shards, reduce_device)
    }
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

/// A pipeline's ordered stages, each paired with the layer range it executes.
pub(crate) type PipelineStages = [(Box<dyn PipelineStageExecutor>, (u32, u32))];

/// Runs an ordered sequence of pipeline stages against a single input,
/// handing the activation off between stages via `transports[i]` after stage
/// `i` runs. `transports.len()` must equal `stages.len() - 1`.
pub(crate) fn run_pipeline(
    stages: &PipelineStages,
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
        Self {
            depth: depth.max(1),
            in_flight: 0,
        }
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
    stages: &PipelineStages,
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
    let data = tensor
        .flatten_all()?
        .to_dtype(candle_core::DType::F32)?
        .to_vec1::<f32>()?;
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
        Self {
            expert_device_index,
        }
    }

    pub(crate) fn device_index_for(&self, expert_id: u32) -> Option<usize> {
        self.expert_device_index.get(&expert_id).copied()
    }

    /// Builds a plan from an explicit expert id -> device-ids index map (e.g.
    /// `hardware_strategy.expert_device_map`), falling back to
    /// `round_robin` for any expert id the map omits, so a deployment can pin
    /// a subset of experts without having to enumerate all of them.
    pub(crate) fn from_explicit_map_or_round_robin(
        expert_device_map: &[(u32, u32)],
        expert_count: u32,
        device_count: usize,
    ) -> Self {
        let mut plan = Self::round_robin(expert_count, device_count);
        for &(expert_id, device_index) in expert_device_map {
            plan.expert_device_index
                .insert(expert_id, device_index as usize);
        }
        plan
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
    let mut routed: std::collections::BTreeMap<usize, Vec<usize>> =
        std::collections::BTreeMap::new();
    for selection in selections {
        if let Some(device_index) = plan.device_index_for(selection.expert_id) {
            routed
                .entry(device_index)
                .or_default()
                .push(selection.token_index);
        }
    }
    routed
}

/// Scans a checkpoint's tensor names for the `<prefix>.layers.<layer_idx>.*.experts.<id>.*`
/// naming convention used by Mixtral/Qwen-MoE-style checkpoints to detect
/// whether a given layer is a dense layer or an MoE layer, and if the
/// latter, how many experts it declares. Returns `None` for a dense layer
/// (no `.experts.` tensors found for that layer index), which callers use as
/// the signal to fall back to the existing dense execution path unchanged.
pub(crate) fn detect_expert_count<'a>(
    tensor_names: impl Iterator<Item = &'a str>,
    layer_idx: usize,
) -> Option<usize> {
    let layer_prefix = format!(".layers.{layer_idx}.");
    let mut max_expert_id: Option<usize> = None;
    for name in tensor_names {
        let Some(layer_pos) = name.find(&layer_prefix) else {
            continue;
        };
        let after_layer = &name[layer_pos + layer_prefix.len()..];
        let Some(experts_pos) = after_layer.find(".experts.") else {
            continue;
        };
        let after_experts = &after_layer[experts_pos + ".experts.".len()..];
        let Some(id_str) = after_experts.split('.').next() else {
            continue;
        };
        if let Ok(id) = id_str.parse::<usize>() {
            max_expert_id = Some(max_expert_id.map_or(id, |current| current.max(id)));
        }
    }
    max_expert_id.map(|id| id + 1)
}

/// A single expert's SwiGLU MLP (Mixtral convention: `w2(silu(w1(x)) * w3(x))`),
/// resident on whichever device `ExpertPlacementPlan` assigned it to.
pub(crate) struct ExpertMlp {
    w1: Linear,
    w3: Linear,
    w2: Linear,
    device: Device,
}

impl ExpertMlp {
    fn load(
        vb: VarBuilder,
        hidden_size: usize,
        intermediate_size: usize,
        device: &Device,
    ) -> CandleResult<Self> {
        Ok(Self {
            w1: linear(hidden_size, intermediate_size, vb.pp("w1"))?,
            w3: linear(hidden_size, intermediate_size, vb.pp("w3"))?,
            w2: linear(intermediate_size, hidden_size, vb.pp("w2"))?,
            device: device.clone(),
        })
    }

    fn forward(&self, x: &Tensor) -> CandleResult<Tensor> {
        let gate = candle_nn::ops::silu(&self.w1.forward(x)?)?;
        let up = self.w3.forward(x)?;
        self.w2.forward(&(gate * up)?)
    }
}

/// Expert-parallel MoE MLP: a replicated gate (top-1 routing) plus one
/// `ExpertMlp` per expert, each pinned to a device per `ExpertPlacementPlan`.
/// `forward` performs genuine gate-then-route dispatch — every token is
/// gathered into a per-expert batch and only that expert's MLP runs over
/// only its assigned tokens, rather than every expert running over every
/// token (dense replication). This is the building block
/// [`super::expert_parallel_llama::ExpertParallelLlama`] uses for each
/// MoE-classified layer of a loaded Mixtral-style checkpoint, alongside
/// `TensorParallelMlp` for that checkpoint's dense-classified layers.
pub(crate) struct ExpertParallelMlp {
    gate: Tensor,
    experts: Vec<ExpertMlp>,
}

impl ExpertParallelMlp {
    pub(crate) fn load(
        vb: VarBuilder,
        hidden_size: usize,
        intermediate_size: usize,
        num_experts: usize,
        plan: &ExpertPlacementPlan,
        devices: &[Device],
    ) -> CandleResult<Self> {
        let gate = vb.pp("gate").get((num_experts, hidden_size), "weight")?;
        let mut experts = Vec::with_capacity(num_experts);
        for expert_id in 0..num_experts {
            let device_index = plan.device_index_for(expert_id as u32).unwrap_or(0);
            let device = &devices[device_index.min(devices.len() - 1)];
            experts.push(ExpertMlp::load(
                vb.pp("experts").pp(expert_id),
                hidden_size,
                intermediate_size,
                device,
            )?);
        }
        Ok(Self { gate, experts })
    }

    /// `x` is a 2-D `[tokens, hidden]` activation on the gate's (primary)
    /// device. Returns the routed `[tokens, hidden]` output in the original
    /// token order.
    pub(crate) fn forward(&self, x: &Tensor) -> CandleResult<Tensor> {
        let (num_tokens, _hidden) = x.dims2()?;
        let num_experts = self.experts.len();
        let logits = x.matmul(&self.gate.t()?)?;
        let logits: Vec<f32> = logits.to_dtype(DType::F32)?.flatten_all()?.to_vec1()?;

        let mut tokens_by_expert: std::collections::BTreeMap<usize, Vec<usize>> =
            std::collections::BTreeMap::new();
        for token_index in 0..num_tokens {
            let row = &logits[token_index * num_experts..(token_index + 1) * num_experts];
            let (expert_id, _) = row
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).expect("gate logits are finite"))
                .expect("at least one expert");
            tokens_by_expert
                .entry(expert_id)
                .or_default()
                .push(token_index);
        }

        let mut rows_by_token: Vec<Option<Tensor>> = vec![None; num_tokens];
        for (expert_id, token_indices) in tokens_by_expert {
            let expert = &self.experts[expert_id];
            let idx = Tensor::from_vec(
                token_indices.iter().map(|&i| i as u32).collect::<Vec<_>>(),
                token_indices.len(),
                x.device(),
            )?;
            let gathered = x.index_select(&idx, 0)?.to_device(&expert.device)?;
            let computed = expert.forward(&gathered)?.to_device(x.device())?;
            for (row_idx, &token_index) in token_indices.iter().enumerate() {
                rows_by_token[token_index] = Some(computed.i(row_idx)?);
            }
        }

        let rows: Vec<Tensor> = rows_by_token
            .into_iter()
            .map(|row| row.expect("every token routed to exactly one expert"))
            .collect();
        Tensor::stack(&rows, 0)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
    fn column_parallel_shard_rejects_uneven_out_features() {
        let w = weight(5, 4, 9); // out_features=5 does not divide evenly by 2 devices.
        let devices = vec![cpu(), cpu()];
        assert!(ColumnParallelLinear::shard(&w, &devices).is_err());
    }

    #[test]
    fn row_parallel_shard_rejects_uneven_in_features() {
        let w = weight(4, 5, 10); // in_features=5 does not divide evenly by 2 devices.
        let devices = vec![cpu(), cpu()];
        assert!(RowParallelLinear::shard(&w, &devices).is_err());
    }

    #[test]
    fn split_for_row_parallel_rejects_uneven_features() {
        let device = cpu();
        let x = Tensor::from_vec(vec![1.0f32; 5], (1, 5), &device).unwrap();
        let devices = vec![cpu(), cpu()];
        assert!(split_for_row_parallel(&x, &devices).is_err());
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

    /// Real GPU-proof: exercises `cudarc::nccl::Comm::all_reduce` against
    /// real CUDA hardware via `ncclCommInitAll` loopback ranks (two ranks on
    /// the single physical GPU the `arc-gpu-runners` CI runner exposes), and
    /// asserts the result matches the existing CPU-staged-sum reference.
    /// Skipped, not failed, on a `candle-cuda` build executed on a host with
    /// zero CUDA devices.
    #[cfg(feature = "candle-cuda")]
    #[test]
    fn nccl_all_reduce_matches_cpu_staged_reference() {
        let device = match Device::cuda_if_available(0) {
            Ok(d @ Device::Cuda(_)) => d,
            _ => {
                eprintln!(
                    "skipping nccl_all_reduce_matches_cpu_staged_reference: no CUDA device available"
                );
                return;
            }
        };
        // Modern NCCL rejects multiple ranks sharing one GPU ordinal (the
        // "loopback" pattern), so this needs 2 distinct physical GPUs to
        // exercise the real `ncclCommInitAll` tensor-parallel path.
        let device1 = match Device::new_cuda(1) {
            Ok(d) => d,
            Err(_) => {
                eprintln!(
                    "skipping nccl_all_reduce_matches_cpu_staged_reference: fewer than 2 CUDA devices available"
                );
                return;
            }
        };

        let w = weight(4, 8, 2); // out_features=4, in_features=8
        let x = Tensor::from_vec(
            vec![1.0f32, -1.0, 0.5, 2.0, 0.25, -0.75, 1.5, -2.0],
            (1, 8),
            &cpu(),
        )
        .unwrap();

        // CPU-staged reference: the same fallback math, on Device::Cpu.
        let cpu_devices = vec![cpu(), cpu()];
        let cpu_sharded = RowParallelLinear::shard(&w, &cpu_devices).unwrap();
        let cpu_x_shards = split_for_row_parallel(&x, &cpu_devices).unwrap();
        let reference = cpu_sharded.forward(&cpu_x_shards, &cpu()).unwrap();

        // Real NCCL all-reduce across 2 distinct physical GPUs.
        let devices = vec![device.clone(), device1];
        let group = NcclShardGroup::try_new(&devices)
            .expect("NCCL communicator init should succeed with a CUDA device available");
        let gpu_sharded = RowParallelLinear::shard(&w, &devices).unwrap();
        let reducer = NcclReducer::new(Some(std::sync::Arc::new(group)));
        let gpu_x_shards = split_for_row_parallel(&x, &devices).unwrap();
        let reduced = gpu_sharded
            .forward_with_reducer(&gpu_x_shards, &device, &reducer)
            .unwrap();

        let reference: Vec<f32> = reference.flatten_all().unwrap().to_vec1().unwrap();
        let reduced: Vec<f32> = reduced
            .to_device(&cpu())
            .unwrap()
            .flatten_all()
            .unwrap()
            .to_vec1()
            .unwrap();
        assert_eq!(reference.len(), reduced.len());
        for (a, b) in reference.iter().zip(reduced.iter()) {
            assert!((a - b).abs() < 1e-4, "{a} != {b}");
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

        let stages: Vec<(Box<dyn PipelineStageExecutor>, (u32, u32))> =
            vec![(Box::new(stage0), (0, 1)), (Box::new(stage1), (2, 3))];
        let transports: Vec<Box<dyn StageTransport>> = vec![Box::new(InProcessTransport {
            next_device: device.clone(),
        })];

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
        let stages: Vec<(Box<dyn PipelineStageExecutor>, (u32, u32))> =
            vec![(Box::new(stage0), (0, 1)), (Box::new(stage1), (2, 3))];
        let transports: Vec<Box<dyn StageTransport>> = vec![Box::new(InProcessTransport {
            next_device: device.clone(),
        })];

        let micro_batches: Vec<Tensor> = (0..5)
            .map(|i| Tensor::from_vec(vec![i as f32], (1, 1), &device).unwrap())
            .collect();
        let expected: Vec<f32> = micro_batches
            .iter()
            .map(|x| {
                ((x * 2.0).unwrap() + 1.0)
                    .unwrap()
                    .to_vec2::<f32>()
                    .unwrap()[0][0]
            })
            .collect();

        // Depth 2: never more than 2 microbatches in flight, but every
        // microbatch must still reach the same output as the single-batch
        // `run_pipeline` reference.
        let mut gate = PipelineDepthGate::new(2);
        let outputs =
            run_pipeline_microbatched(&stages, &transports, micro_batches, &mut gate).unwrap();
        assert_eq!(
            gate.in_flight(),
            0,
            "every microbatch must release its slot on completion"
        );

        let outputs: Vec<f32> = outputs
            .iter()
            .map(|t| t.to_vec2::<f32>().unwrap()[0][0])
            .collect();
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

    #[test]
    fn nccl_tcp_rendezvous_broadcasts_unique_id_bytes() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let mut expected = [0u8; NCCL_UNIQUE_ID_LEN];
        for (i, byte) in expected.iter_mut().enumerate() {
            *byte = (i % 251) as u8;
        }

        let server = std::thread::spawn(move || {
            broadcast_nccl_rendezvous_bytes(&listener, &expected, 2).unwrap();
            expected
        });

        let first = fetch_nccl_rendezvous_bytes(addr).unwrap();
        let second = fetch_nccl_rendezvous_bytes(addr).unwrap();
        let expected = server.join().unwrap();
        assert_eq!(first, expected);
        assert_eq!(second, expected);
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
    fn explicit_map_overrides_are_applied_on_top_of_round_robin() {
        let plan = ExpertPlacementPlan::from_explicit_map_or_round_robin(&[(2, 1)], 4, 2);
        // Expert 2 is pinned to device 1 by the explicit map, overriding what
        // round-robin would have assigned it (device 0).
        assert_eq!(plan.device_index_for(2), Some(1));
        // Every other expert keeps its round-robin placement.
        assert_eq!(plan.device_index_for(0), Some(0));
        assert_eq!(plan.device_index_for(1), Some(1));
        assert_eq!(plan.device_index_for(3), Some(1));
    }

    #[test]
    fn explicit_map_falls_back_to_round_robin_for_omitted_experts() {
        let plan = ExpertPlacementPlan::from_explicit_map_or_round_robin(&[], 4, 2);
        assert_eq!(plan, ExpertPlacementPlan::round_robin(4, 2));
    }

    #[test]
    fn tokens_are_routed_only_to_their_selected_experts_device() {
        let plan = ExpertPlacementPlan::round_robin(2, 2);
        let selections = vec![
            TokenExpertSelection {
                token_index: 0,
                expert_id: 0,
            },
            TokenExpertSelection {
                token_index: 1,
                expert_id: 1,
            },
            TokenExpertSelection {
                token_index: 2,
                expert_id: 0,
            },
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

    #[test]
    fn detect_expert_count_finds_the_highest_expert_id_for_a_moe_layer() {
        let names = [
            "model.layers.0.block_sparse_moe.gate.weight",
            "model.layers.0.block_sparse_moe.experts.0.w1.weight",
            "model.layers.0.block_sparse_moe.experts.0.w2.weight",
            "model.layers.0.block_sparse_moe.experts.1.w1.weight",
            "model.layers.0.block_sparse_moe.experts.2.w1.weight",
            "model.layers.1.mlp.gate_proj.weight",
        ];
        assert_eq!(detect_expert_count(names.iter().copied(), 0), Some(3));
    }

    #[test]
    fn detect_expert_count_returns_none_for_a_dense_layer() {
        let names = [
            "model.layers.0.mlp.gate_proj.weight",
            "model.layers.0.mlp.up_proj.weight",
            "model.layers.0.mlp.down_proj.weight",
        ];
        assert_eq!(detect_expert_count(names.iter().copied(), 0), None);
    }

    fn random_tensor(rows: usize, cols: usize, device: &Device) -> Tensor {
        let data: Vec<f32> = (0..rows * cols)
            .map(|i| ((i as f32) * 0.037).sin())
            .collect();
        Tensor::from_vec(data, (rows, cols), device).unwrap()
    }

    fn load_expert_parallel_mlp(
        hidden: usize,
        intermediate: usize,
        num_experts: usize,
        devices: &[Device],
    ) -> (ExpertParallelMlp, std::collections::HashMap<String, Tensor>) {
        let device = devices[0].clone();
        let mut weights = std::collections::HashMap::new();
        weights.insert(
            "gate.weight".to_string(),
            random_tensor(num_experts, hidden, &device),
        );
        for expert_id in 0..num_experts {
            for (suffix, (out_dim, in_dim)) in [
                ("w1", (intermediate, hidden)),
                ("w3", (intermediate, hidden)),
                ("w2", (hidden, intermediate)),
            ] {
                weights.insert(
                    format!("experts.{expert_id}.{suffix}.weight"),
                    random_tensor(out_dim, in_dim, &device),
                );
            }
        }
        let vb = VarBuilder::from_tensors(weights.clone(), DType::F32, &device);
        let plan = ExpertPlacementPlan::round_robin(num_experts as u32, devices.len());
        let mlp =
            ExpertParallelMlp::load(vb, hidden, intermediate, num_experts, &plan, devices).unwrap();
        (mlp, weights)
    }

    #[test]
    fn expert_parallel_mlp_matches_per_token_dense_dispatch_reference() {
        let hidden = 8;
        let intermediate = 16;
        let num_experts = 4;
        let num_tokens = 6;
        let device = cpu();
        // Two simulated devices, both CPU (this build has no live CUDA
        // backend — see Task 2's caveat), so experts 0/2 land on device 0
        // and experts 1/3 land on device 1, exercising real cross-device
        // gather/scatter even though both "devices" are the same backend.
        let devices = vec![device.clone(), device.clone()];
        let (mlp, weights) = load_expert_parallel_mlp(hidden, intermediate, num_experts, &devices);

        let x = random_tensor(num_tokens, hidden, &device);
        let batched = mlp.forward(&x).unwrap();

        // Reference: route each token to its top-1 expert exactly as
        // `ExpertParallelMlp::forward` would, but compute every token's
        // expert MLP one row at a time directly from the same weights,
        // proving the grouped/multi-device computation is numerically
        // identical to literal per-token sparse dispatch.
        let gate = weights.get("gate.weight").unwrap();
        let logits = x.matmul(&gate.t().unwrap()).unwrap();
        let logits: Vec<f32> = logits.flatten_all().unwrap().to_vec1().unwrap();
        let mut reference_rows = Vec::with_capacity(num_tokens);
        for token in 0..num_tokens {
            let row = &logits[token * num_experts..(token + 1) * num_experts];
            let (expert_id, _) = row
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
                .unwrap();
            let w1 = weights
                .get(&format!("experts.{expert_id}.w1.weight"))
                .unwrap();
            let w3 = weights
                .get(&format!("experts.{expert_id}.w3.weight"))
                .unwrap();
            let w2 = weights
                .get(&format!("experts.{expert_id}.w2.weight"))
                .unwrap();
            let token_row = x.i(token).unwrap().reshape((1, hidden)).unwrap();
            let gate_out =
                candle_nn::ops::silu(&token_row.matmul(&w1.t().unwrap()).unwrap()).unwrap();
            let up_out = token_row.matmul(&w3.t().unwrap()).unwrap();
            let out = (gate_out * up_out)
                .unwrap()
                .matmul(&w2.t().unwrap())
                .unwrap();
            reference_rows.push(out);
        }
        let reference = Tensor::cat(&reference_rows, 0).unwrap();

        let batched_vec: Vec<f32> = batched.flatten_all().unwrap().to_vec1().unwrap();
        let reference_vec: Vec<f32> = reference.flatten_all().unwrap().to_vec1().unwrap();
        assert_eq!(batched_vec.len(), reference_vec.len());
        for (a, b) in batched_vec.iter().zip(reference_vec.iter()) {
            assert!((a - b).abs() < 1e-4, "{a} vs {b}");
        }
    }
}
