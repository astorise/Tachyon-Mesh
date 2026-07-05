use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::{Mutex, OnceLock},
    time::Duration,
};

const DISPATCH_TOTAL_HELP: &str =
    "Total number of inter-FaaS mesh dispatch decisions, broken down by transport mode and reason.";
const DISPATCH_DURATION_HELP: &str =
    "Inter-FaaS mesh dispatch duration in seconds, broken down by transport mode.";

#[cfg_attr(not(unix), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum MeshDispatchMode {
    InProcess,
    Uds,
    Tcp,
}

impl MeshDispatchMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::InProcess => "in_process",
            Self::Uds => "uds",
            Self::Tcp => "tcp",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum MeshDispatchReason {
    Ok,
    Saturated,
    Pressure,
    Remote,
}

impl MeshDispatchReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Saturated => "saturated",
            Self::Pressure => "pressure",
            Self::Remote => "remote",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshDispatchMetricsSnapshot {
    pub(crate) totals: Vec<MeshDispatchTotalSnapshot>,
    pub(crate) durations: Vec<MeshDispatchDurationSnapshot>,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshDispatchTotalSnapshot {
    pub(crate) mode: String,
    pub(crate) reason: String,
    pub(crate) total: u64,
}

#[derive(Clone, Debug, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MeshDispatchDurationSnapshot {
    pub(crate) mode: String,
    pub(crate) count: u64,
    pub(crate) avg_latency_ms: f64,
}

#[derive(Default)]
struct MeshDispatchMetricState {
    totals: BTreeMap<(MeshDispatchMode, MeshDispatchReason), u64>,
    duration_totals_us: BTreeMap<MeshDispatchMode, u64>,
    duration_counts: BTreeMap<MeshDispatchMode, u64>,
}

static DISPATCH_STATE: OnceLock<Mutex<MeshDispatchMetricState>> = OnceLock::new();
static DISPATCH_TOTAL_VEC: OnceLock<Option<prometheus::CounterVec>> = OnceLock::new();
static DISPATCH_DURATION_VEC: OnceLock<Option<prometheus::HistogramVec>> = OnceLock::new();

fn dispatch_state() -> &'static Mutex<MeshDispatchMetricState> {
    DISPATCH_STATE.get_or_init(|| Mutex::new(MeshDispatchMetricState::default()))
}

fn dispatch_total_vec() -> Option<&'static prometheus::CounterVec> {
    DISPATCH_TOTAL_VEC
        .get_or_init(|| {
            let cv = prometheus::CounterVec::new(
                prometheus::Opts::new("faas_mesh_dispatch_total", DISPATCH_TOTAL_HELP),
                &["mode", "reason"],
            )
            .ok()?;
            let _ = prometheus::default_registry().register(Box::new(cv.clone()));
            Some(cv)
        })
        .as_ref()
}

fn dispatch_duration_vec() -> Option<&'static prometheus::HistogramVec> {
    DISPATCH_DURATION_VEC
        .get_or_init(|| {
            let histogram = prometheus::HistogramVec::new(
                prometheus::HistogramOpts::new(
                    "faas_mesh_dispatch_duration_seconds",
                    DISPATCH_DURATION_HELP,
                ),
                &["mode"],
            )
            .ok()?;
            let _ = prometheus::default_registry().register(Box::new(histogram.clone()));
            Some(histogram)
        })
        .as_ref()
}

pub(crate) fn record_mesh_dispatch(
    mode: MeshDispatchMode,
    reason: MeshDispatchReason,
    duration: Duration,
) {
    // Debug-level only: lets the bench regression harness (see `bench/`)
    // confirm a scenario stayed in-process by grepping `kubectl logs`
    // instead of requiring authenticated access to `/admin/metrics`.
    tracing::debug!(
        mode = mode.as_str(),
        reason = reason.as_str(),
        duration_ms = duration.as_secs_f64() * 1_000.0,
        "mesh dispatch decision"
    );
    if let Some(counter) = dispatch_total_vec() {
        counter
            .with_label_values(&[mode.as_str(), reason.as_str()])
            .inc();
    }
    if let Some(histogram) = dispatch_duration_vec() {
        histogram
            .with_label_values(&[mode.as_str()])
            .observe(duration.as_secs_f64());
    }

    let mut state = match dispatch_state().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *state.totals.entry((mode, reason)).or_default() += 1;
    let duration_us = u64::try_from(duration.as_micros()).unwrap_or(u64::MAX);
    let total_us = state.duration_totals_us.entry(mode).or_default();
    *total_us = total_us.saturating_add(duration_us);
    *state.duration_counts.entry(mode).or_default() += 1;
}

pub(crate) fn mesh_dispatch_metrics_snapshot() -> MeshDispatchMetricsSnapshot {
    let state = match dispatch_state().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    let totals = state
        .totals
        .iter()
        .map(|((mode, reason), total)| MeshDispatchTotalSnapshot {
            mode: mode.as_str().to_owned(),
            reason: reason.as_str().to_owned(),
            total: *total,
        })
        .collect();
    let durations = state
        .duration_counts
        .iter()
        .map(|(mode, count)| {
            let total_us = state.duration_totals_us.get(mode).copied().unwrap_or(0);
            MeshDispatchDurationSnapshot {
                mode: mode.as_str().to_owned(),
                count: *count,
                avg_latency_ms: if *count == 0 {
                    0.0
                } else {
                    total_us as f64 / *count as f64 / 1_000.0
                },
            }
        })
        .collect();

    MeshDispatchMetricsSnapshot { totals, durations }
}

#[cfg(test)]
pub(crate) fn mesh_dispatch_total_for(mode: MeshDispatchMode, reason: MeshDispatchReason) -> u64 {
    let state = match dispatch_state().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    state.totals.get(&(mode, reason)).copied().unwrap_or(0)
}
