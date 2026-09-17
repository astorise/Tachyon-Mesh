//! Host-level accelerator capability reporting: ties `npu`/`tpu` (and `gpu`)
//! *availability* to whether a real execution backend was actually probed and
//! found on this host, rather than to whether the accelerator class is a
//! valid scheduling/QoS lane (that pre-existing, separate concept is
//! `AiInferenceRuntime::supports_accelerator`/`AcceleratorKind::ALL`, used to
//! size per-class request queues and unchanged by this module).
//!
//! NPU and TPU availability is delegated to optional OpenVINO and Edge TPU
//! runners. A runner is available only after its probe confirms the expected
//! physical device; otherwise dispatch can explicitly fall back to CPU.
//!
//! `Gpu` is probed for real via `parallel::discover_cluster_topology`, which
//! enumerates real CUDA ordinals on `candle-cuda` builds (and reports only
//! the CPU device otherwise) — so `Gpu` honestly reports `Unavailable` on a
//! host with no CUDA device, instead of being assumed present.

use super::AcceleratorKind;

/// Why an accelerator class is unavailable. Stable, machine-checkable reason
/// strings (not free text) so callers can branch on the cause.
pub(crate) const DEVICE_NOT_DETECTED: &str = "device_not_detected";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AvailabilityStatus {
    Available,
    Unavailable { reason: &'static str },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AcceleratorAvailability {
    pub(crate) backend: AcceleratorKind,
    pub(crate) status: AvailabilityStatus,
}

impl AcceleratorAvailability {
    pub(crate) fn is_available(&self) -> bool {
        matches!(self.status, AvailabilityStatus::Available)
    }
}

/// Probes whether `backend` has a real, initialized execution path on this
/// host. `Cpu` is always available (the host itself). `Gpu` reflects real
/// CUDA device enumeration. `Npu`/`Tpu` reflect the corresponding optional
/// vendor runner probe.
pub(crate) fn probe(backend: AcceleratorKind) -> AcceleratorAvailability {
    let status = match backend {
        // The network lane needs no local hardware: its "device" is another
        // process, and whether that process is reachable is a per-request
        // outcome rather than a host capability.
        AcceleratorKind::Cpu | AcceleratorKind::Network => AvailabilityStatus::Available,
        AcceleratorKind::Gpu => {
            if super::parallel::discover_cluster_topology().devices.len() > 1 {
                AvailabilityStatus::Available
            } else {
                AvailabilityStatus::Unavailable {
                    reason: DEVICE_NOT_DETECTED,
                }
            }
        }
        AcceleratorKind::Npu | AcceleratorKind::Tpu => {
            if super::vendor_accelerator::is_available(backend) {
                AvailabilityStatus::Available
            } else {
                AvailabilityStatus::Unavailable {
                    reason: DEVICE_NOT_DETECTED,
                }
            }
        }
    };
    AcceleratorAvailability { backend, status }
}

/// Resolves which accelerator class a dispatch should actually target: the
/// `preferred` class if `probe` reports it available, otherwise `fallback`.
/// Mirrors the fallback policy `heterogeneous-accelerator-orchestration`
/// already specifies at the config level, made testable without hardware by
/// taking the probe function as a parameter.
pub(crate) fn resolve_with_fallback(
    preferred: AcceleratorKind,
    fallback: AcceleratorKind,
    probe_fn: impl Fn(AcceleratorKind) -> AcceleratorAvailability,
) -> AcceleratorKind {
    if probe_fn(preferred).is_available() {
        preferred
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_is_always_available() {
        assert_eq!(
            probe(AcceleratorKind::Cpu).status,
            AvailabilityStatus::Available
        );
    }

    #[test]
    fn npu_and_tpu_report_unavailable_without_initialized_vendor_runner() {
        for kind in [AcceleratorKind::Npu, AcceleratorKind::Tpu] {
            let availability = probe(kind);
            assert_eq!(availability.backend, kind);
            assert_eq!(
                availability.status,
                AvailabilityStatus::Unavailable {
                    reason: DEVICE_NOT_DETECTED
                }
            );
            assert!(!availability.is_available());
        }
    }

    #[test]
    fn gpu_reports_unavailable_on_a_cpu_only_build() {
        // This test suite runs without the `candle-cuda` feature, so
        // `discover_cluster_topology` reports only the CPU device and GPU
        // should honestly report unavailable rather than assumed-present.
        let availability = probe(AcceleratorKind::Gpu);
        assert_eq!(
            availability.status,
            AvailabilityStatus::Unavailable {
                reason: DEVICE_NOT_DETECTED
            }
        );
    }

    #[test]
    fn resolve_with_fallback_picks_preferred_when_available() {
        let resolved = resolve_with_fallback(AcceleratorKind::Cpu, AcceleratorKind::Gpu, probe);
        assert_eq!(resolved, AcceleratorKind::Cpu);
    }

    #[test]
    fn resolve_with_fallback_falls_back_when_preferred_is_unavailable() {
        let resolved = resolve_with_fallback(AcceleratorKind::Npu, AcceleratorKind::Cpu, probe);
        assert_eq!(resolved, AcceleratorKind::Cpu);
    }

    #[test]
    fn resolve_with_fallback_honors_an_injected_probe_for_testability() {
        // Demonstrates the fallback logic is independent of real hardware:
        // a caller can inject any probe function, e.g. to simulate an NPU
        // backend becoming available once Task 2/3 land.
        let always_available = |backend| AcceleratorAvailability {
            backend,
            status: AvailabilityStatus::Available,
        };
        let resolved =
            resolve_with_fallback(AcceleratorKind::Npu, AcceleratorKind::Cpu, always_available);
        assert_eq!(resolved, AcceleratorKind::Npu);
    }
}
