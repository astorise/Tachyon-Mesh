//! Host-level accelerator capability reporting: ties `npu`/`tpu` (and `gpu`)
//! *availability* to whether a real execution backend was actually probed and
//! found on this host, rather than to whether the accelerator class is a
//! valid scheduling/QoS lane (that pre-existing, separate concept is
//! `AiInferenceRuntime::supports_accelerator`/`AcceleratorKind::ALL`, used to
//! size per-class request queues and unchanged by this module).
//!
//! Today there is no NPU runtime (e.g. OpenVINO) or TPU runtime (e.g.
//! `libedgetpu`/Coral Edge TPU) wired into this host at all — `Npu`/`Tpu`
//! always report `Unavailable { reason: NO_BACKEND_WIRED }`. Wiring a real
//! vendor SDK backend for either class, and the physical hardware validation
//! that would accompany it (a Coral USB TPU, an NPU-equipped machine), is out
//! of scope for this change: it requires vendor SDK dependencies and physical
//! test hardware that are not available in this environment. See
//! `openspec/changes/2026-06-19-npu-tpu-real-device-execution`.
//!
//! `Gpu` is probed for real via `parallel::discover_cluster_topology`, which
//! enumerates real CUDA ordinals on `candle-cuda` builds (and reports only
//! the CPU device otherwise) — so `Gpu` honestly reports `Unavailable` on a
//! host with no CUDA device, instead of being assumed present.

use super::AcceleratorKind;

/// Why an accelerator class is unavailable. Stable, machine-checkable reason
/// strings (not free text) so callers can branch on the cause.
pub(crate) const NO_BACKEND_WIRED: &str = "no_backend_wired";
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
/// CUDA device enumeration. `Npu`/`Tpu` are always `Unavailable`: no vendor
/// SDK backend exists yet (see module docs).
pub(crate) fn probe(backend: AcceleratorKind) -> AcceleratorAvailability {
    let status = match backend {
        AcceleratorKind::Cpu => AvailabilityStatus::Available,
        AcceleratorKind::Gpu => {
            if super::parallel::discover_cluster_topology().devices.len() > 1 {
                AvailabilityStatus::Available
            } else {
                AvailabilityStatus::Unavailable {
                    reason: DEVICE_NOT_DETECTED,
                }
            }
        }
        AcceleratorKind::Npu | AcceleratorKind::Tpu => AvailabilityStatus::Unavailable {
            reason: NO_BACKEND_WIRED,
        },
    };
    AcceleratorAvailability {
        backend,
        status,
    }
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
        assert_eq!(probe(AcceleratorKind::Cpu).status, AvailabilityStatus::Available);
    }

    #[test]
    fn npu_and_tpu_report_unavailable_with_no_backend_wired() {
        for kind in [AcceleratorKind::Npu, AcceleratorKind::Tpu] {
            let availability = probe(kind);
            assert_eq!(availability.backend, kind);
            assert_eq!(
                availability.status,
                AvailabilityStatus::Unavailable {
                    reason: NO_BACKEND_WIRED
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
        let resolved = resolve_with_fallback(AcceleratorKind::Npu, AcceleratorKind::Cpu, always_available);
        assert_eq!(resolved, AcceleratorKind::Npu);
    }
}
