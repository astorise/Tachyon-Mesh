//! Pure (no GPU runtime dependency) types and validation logic for
//! tensor/pipeline/expert-parallel execution plans. Shared between
//! `core-host` (which validates a plan against *discovered* hardware
//! topology before dispatching real Candle execution) and
//! `system-faas-config-api` (which validates a plan's internal shape at
//! `apply-model-deployment` time, before any hardware has been consulted).
//!
//! Mirrors `parallel-strategy`/`parallel-execution-plan`/`topology-error` in
//! `wit/ai/inference.wit`.

use std::fmt;

/// How a model's forward pass is split across more than one accelerator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ParallelStrategy {
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
pub enum InterconnectClass {
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
pub struct DeviceInfo {
    pub device_id: u32,
    pub free_vram_bytes: u64,
}

/// The cluster's discovered hardware topology, as known at deployment-validation
/// time. `interconnect` reports the worst-case (most constrained) interconnect
/// class across the participating device set, which is sufficient to gate a
/// tensor-parallel plan: a plan is only as fast as its slowest link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterTopology {
    pub devices: Vec<DeviceInfo>,
    pub interconnect: InterconnectClass,
}

impl ClusterTopology {
    pub fn device(&self, device_id: u32) -> Option<&DeviceInfo> {
        self.devices.iter().find(|d| d.device_id == device_id)
    }
}

/// A requested execution plan, validated against `ClusterTopology` before any
/// weights are loaded. Mirrors `parallel-execution-plan` in
/// `wit/ai/inference.wit`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParallelExecutionPlan {
    pub strategy: ParallelStrategy,
    pub device_ids: Vec<u32>,
    /// For pipeline-parallel: inclusive (start, end) layer range per device,
    /// indexed by position in `device_ids`.
    pub stage_layer_ranges: Vec<(u32, u32)>,
    /// For expert-parallel: (expert_id, device_ids index) placement pairs.
    pub expert_device_map: Vec<(u32, u32)>,
    /// Bytes of VRAM required per shard/device, computed from the model's
    /// size and the chosen strategy. Required for `VramPerShardExceeded`
    /// validation; zero means "not yet sized" and is never rejected on VRAM
    /// grounds.
    pub required_vram_bytes_per_device: u64,
    pub pipeline_depth: u32,
}

/// Typed reasons a plan cannot be satisfied by the cluster's discovered
/// hardware topology. Mirrors `topology-error` in `wit/ai/inference.wit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TopologyError {
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
pub const TENSOR_PARALLEL_MIN_INTERCONNECT: InterconnectClass = InterconnectClass::Pcie;

/// Validates that `plan` can be satisfied by `topology` without loading or
/// sharding a model. Requires live hardware topology (device count,
/// interconnect, free VRAM); use `validate_plan_shape` instead where only
/// the plan itself (not discovered hardware) is available.
pub fn validate_parallel_topology(
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
        for device_id in &plan.device_ids {
            if let Some(device) = topology.device(*device_id) {
                if plan.required_vram_bytes_per_device > device.free_vram_bytes {
                    return Err(TopologyError::VramPerShardExceeded {
                        required_bytes: plan.required_vram_bytes_per_device,
                        available_bytes: device.free_vram_bytes,
                    });
                }
            }
        }
    }

    Ok(())
}

/// Validates a plan's internal shape (device-count sanity, pipeline stage
/// coverage, expert-device-map bounds) without consulting live hardware
/// topology. Run at deployment-submission time (`apply-model-deployment`),
/// before a `ClusterTopology` is available; `validate_parallel_topology`
/// performs the complementary hardware-aware check once the host actually
/// admits the deployment for execution.
pub fn validate_plan_shape(plan: &ParallelExecutionPlan) -> Result<(), String> {
    if plan.strategy == ParallelStrategy::None {
        return Ok(());
    }

    if plan.device_ids.len() < 2 {
        return Err(format!(
            "{:?} requires at least 2 device-ids, got {}",
            plan.strategy,
            plan.device_ids.len()
        ));
    }

    match plan.strategy {
        ParallelStrategy::PipelineParallel => {
            if plan.stage_layer_ranges.len() != plan.device_ids.len() {
                return Err(format!(
                    "pipeline-parallel requires one stage-layer-range per device-id: {} ranges for {} devices",
                    plan.stage_layer_ranges.len(),
                    plan.device_ids.len()
                ));
            }
            let mut previous_end: Option<u32> = None;
            for &(start, end) in &plan.stage_layer_ranges {
                if start > end {
                    return Err(format!(
                        "pipeline-parallel stage range ({start}, {end}) has start > end"
                    ));
                }
                if let Some(prev) = previous_end {
                    if start != prev + 1 {
                        return Err(format!(
                            "pipeline-parallel stage ranges must be contiguous: range starting at {start} does not follow the previous range ending at {prev}"
                        ));
                    }
                }
                previous_end = Some(end);
            }
            if plan.pipeline_depth == 0 {
                return Err("pipeline-parallel requires pipeline-depth >= 1".to_owned());
            }
        }
        ParallelStrategy::ExpertParallel => {
            for &(expert_id, device_index) in &plan.expert_device_map {
                if device_index as usize >= plan.device_ids.len() {
                    return Err(format!(
                        "expert-device-map entry for expert {expert_id} references device index {device_index}, but only {} device-ids are declared",
                        plan.device_ids.len()
                    ));
                }
            }
        }
        ParallelStrategy::TensorParallel | ParallelStrategy::None => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn topology(device_count: usize, interconnect: InterconnectClass, free_vram: u64) -> ClusterTopology {
        ClusterTopology {
            devices: (0..device_count as u32)
                .map(|device_id| DeviceInfo { device_id, free_vram_bytes: free_vram })
                .collect(),
            interconnect,
        }
    }

    fn plan(strategy: ParallelStrategy, device_ids: Vec<u32>) -> ParallelExecutionPlan {
        ParallelExecutionPlan {
            strategy,
            device_ids,
            stage_layer_ranges: Vec::new(),
            expert_device_map: Vec::new(),
            required_vram_bytes_per_device: 0,
            pipeline_depth: 1,
        }
    }

    #[test]
    fn none_strategy_always_validates() {
        let p = plan(ParallelStrategy::None, vec![]);
        let t = topology(1, InterconnectClass::HighBandwidth, 0);
        assert!(validate_parallel_topology(&p, &t).is_ok());
        assert!(validate_plan_shape(&p).is_ok());
    }

    #[test]
    fn insufficient_device_count_is_rejected() {
        let p = plan(ParallelStrategy::TensorParallel, vec![0, 1, 2]);
        let t = topology(2, InterconnectClass::HighBandwidth, 0);
        assert_eq!(
            validate_parallel_topology(&p, &t),
            Err(TopologyError::InsufficientDeviceCount { required: 3, available: 2 })
        );
    }

    #[test]
    fn incompatible_interconnect_is_rejected_for_tensor_parallel() {
        let p = plan(ParallelStrategy::TensorParallel, vec![0, 1]);
        let t = topology(2, InterconnectClass::CrossNode, 0);
        assert_eq!(
            validate_parallel_topology(&p, &t),
            Err(TopologyError::IncompatibleInterconnect {
                required: InterconnectClass::Pcie,
                available: InterconnectClass::CrossNode
            })
        );
    }

    #[test]
    fn shape_check_rejects_single_device_parallel_plan() {
        let p = plan(ParallelStrategy::TensorParallel, vec![0]);
        assert!(validate_plan_shape(&p).is_err());
    }

    #[test]
    fn shape_check_rejects_mismatched_pipeline_stage_count() {
        let mut p = plan(ParallelStrategy::PipelineParallel, vec![0, 1]);
        p.stage_layer_ranges = vec![(0, 5)];
        p.pipeline_depth = 1;
        assert!(validate_plan_shape(&p).is_err());
    }

    #[test]
    fn shape_check_rejects_non_contiguous_pipeline_stage_ranges() {
        let mut p = plan(ParallelStrategy::PipelineParallel, vec![0, 1]);
        p.stage_layer_ranges = vec![(0, 5), (7, 10)];
        p.pipeline_depth = 1;
        assert!(validate_plan_shape(&p).is_err());
    }

    #[test]
    fn shape_check_accepts_well_formed_pipeline_plan() {
        let mut p = plan(ParallelStrategy::PipelineParallel, vec![0, 1]);
        p.stage_layer_ranges = vec![(0, 5), (6, 10)];
        p.pipeline_depth = 4;
        assert!(validate_plan_shape(&p).is_ok());
    }

    #[test]
    fn shape_check_rejects_out_of_bounds_expert_device_map() {
        let mut p = plan(ParallelStrategy::ExpertParallel, vec![0, 1]);
        p.expert_device_map = vec![(0, 0), (1, 5)];
        assert!(validate_plan_shape(&p).is_err());
    }

    #[test]
    fn shape_check_accepts_well_formed_expert_plan() {
        let mut p = plan(ParallelStrategy::ExpertParallel, vec![0, 1]);
        p.expert_device_map = vec![(0, 0), (1, 1), (2, 0)];
        assert!(validate_plan_shape(&p).is_ok());
    }
}
