use serde_json::json;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, MutexGuard, OnceLock, PoisonError,
    },
    time::Instant,
};
use tokio::sync::mpsc;

/// Recover from a poisoned telemetry lock while leaving an audit trail. The lock
/// is intentionally treated as recoverable (we would rather drop one inconsistent
/// snapshot than wedge the request path) but every poison event SHALL be logged
/// so silent corruption never goes unnoticed.
fn recover_poisoned<'a, T>(
    registry: &'static str,
    result: Result<MutexGuard<'a, T>, PoisonError<MutexGuard<'a, T>>>,
) -> MutexGuard<'a, T> {
    match result {
        Ok(guard) => guard,
        Err(poison) => {
            tracing::warn!(
                target: "tachyon::telemetry",
                registry = registry,
                "telemetry registry mutex was poisoned; continuing with recovered guard"
            );
            poison.into_inner()
        }
    }
}

const TELEMETRY_CHANNEL_CAPACITY: usize = 10_000;
const ERROR_STATUS_THRESHOLD: u16 = 400;
const CUSTOM_METRIC_HELP: &str = "Custom Tachyon FaaS metric submitted through WIT telemetry.";

#[derive(Clone)]
pub(crate) struct TelemetryHandle {
    sender: mpsc::Sender<TelemetryEvent>,
    snapshot: TelemetrySnapshotStore,
}

#[derive(Clone)]
struct TelemetrySnapshotStore {
    metrics: Arc<Mutex<AggregatedMetrics>>,
    active_requests: Arc<AtomicUsize>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TelemetrySnapshot {
    pub(crate) total_requests: u64,
    pub(crate) completed_requests: u64,
    pub(crate) error_requests: u64,
    pub(crate) active_requests: u32,
    pub(crate) dropped_events: u64,
    pub(crate) last_status: u16,
    pub(crate) total_duration_us: u64,
    pub(crate) total_wasm_duration_us: u64,
    pub(crate) total_host_overhead_us: u64,
}

#[derive(Debug)]
pub(crate) enum TelemetryEvent {
    RequestStart {
        trace_id: String,
        path: String,
        sampled: bool,
        traceparent: Option<String>,
        timestamp: Instant,
    },
    WasmStart {
        trace_id: String,
        timestamp: Instant,
    },
    WasmEnd {
        trace_id: String,
        timestamp: Instant,
    },
    RequestEnd {
        trace_id: String,
        status: u16,
        fuel_consumed: Option<u64>,
        timestamp: Instant,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CustomMetricType {
    Counter,
    Gauge,
    Histogram,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CustomMetric {
    pub(crate) name: String,
    pub(crate) value: f64,
    pub(crate) metric_type: CustomMetricType,
    pub(crate) tags: Vec<(String, String)>,
}

#[derive(Default)]
struct AggregatedMetrics {
    total_requests: u64,
    completed_requests: u64,
    error_requests: u64,
    dropped_events: u64,
    last_status: u16,
    total_duration_us: u64,
    total_wasm_duration_us: u64,
    total_host_overhead_us: u64,
}

#[derive(Default)]
struct RequestState {
    path: Option<String>,
    sampled: bool,
    traceparent: Option<String>,
    fuel_consumed: Option<u64>,
    request_started_at: Option<Instant>,
    wasm_started_at: Option<Instant>,
    wasm_finished_at: Option<Instant>,
}

struct CompletedRequest {
    line: String,
    sampled: bool,
    status: u16,
    total_duration_us: u64,
    wasm_duration_us: u64,
    host_overhead_us: u64,
}

type TelemetryEmitter = Arc<dyn Fn(String) -> bool + Send + Sync>;

/// Strongly typed handle to a custom-metric Prometheus collector. We deliberately
/// avoid `Box<dyn Any>` here so the registry can be read without unchecked
/// downcasts — every collector exposes the same `record(value, labels)` API.
#[derive(Clone)]
enum CustomCollector {
    Counter(prometheus::CounterVec),
    Gauge(prometheus::GaugeVec),
    Histogram(prometheus::HistogramVec),
}

impl CustomCollector {
    #[cfg(feature = "experimental")]
    fn metric_type(&self) -> CustomMetricType {
        match self {
            Self::Counter(_) => CustomMetricType::Counter,
            Self::Gauge(_) => CustomMetricType::Gauge,
            Self::Histogram(_) => CustomMetricType::Histogram,
        }
    }
}

static CUSTOM_METRICS: OnceLock<Mutex<HashMap<String, f64>>> = OnceLock::new();
static PROMETHEUS_COLLECTORS: OnceLock<Mutex<HashMap<String, CustomCollector>>> = OnceLock::new();

pub(crate) struct ActiveRequestGuard {
    active_requests: Arc<AtomicUsize>,
}

#[cfg(feature = "experimental")]
pub(crate) fn init_telemetry() -> TelemetryHandle {
    init_telemetry_with_emitter(|line| {
        println!("{line}");
        true
    })
}

pub(crate) fn record_event(handle: &TelemetryHandle, event: TelemetryEvent) {
    if handle.sender.try_send(event).is_err() {
        handle.snapshot.record_dropped_event();
    }
}

pub(crate) fn begin_request(handle: &TelemetryHandle) -> ActiveRequestGuard {
    handle
        .snapshot
        .active_requests
        .fetch_add(1, Ordering::SeqCst);

    ActiveRequestGuard {
        active_requests: Arc::clone(&handle.snapshot.active_requests),
    }
}

pub(crate) fn active_requests(handle: &TelemetryHandle) -> usize {
    handle.snapshot.active_requests.load(Ordering::Relaxed)
}

pub(crate) fn snapshot(handle: &TelemetryHandle) -> TelemetrySnapshot {
    handle.snapshot.snapshot()
}

pub(crate) fn push_custom_metric(metric: CustomMetric) -> Result<(), String> {
    let normalized_name = normalize_metric_name(&metric.name)?;
    let label_keys: Vec<String> = metric
        .tags
        .iter()
        .map(|(key, _)| normalize_label_name(key))
        .collect::<Result<_, _>>()?;
    let label_key_refs: Vec<&str> = label_keys.iter().map(String::as_str).collect();
    let label_values: Vec<&str> = metric
        .tags
        .iter()
        .map(|(_, value)| value.as_str())
        .collect();
    let cache_key = collector_key(metric.metric_type, &normalized_name, &label_keys);
    let collectors = PROMETHEUS_COLLECTORS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut collectors = recover_poisoned("prometheus_collectors", collectors.lock());

    match metric.metric_type {
        CustomMetricType::Counter => {
            let collector = match collectors.get(&cache_key) {
                Some(CustomCollector::Counter(counter)) => counter.clone(),
                Some(_) => {
                    return Err(format!(
                        "custom metric `{}` is already registered with a different type",
                        metric.name
                    ));
                }
                None => {
                    let opts = prometheus::Opts::new(&normalized_name, CUSTOM_METRIC_HELP);
                    let counter = prometheus::CounterVec::new(opts, &label_key_refs)
                        .map_err(|error| error.to_string())?;
                    register_collector(Box::new(counter.clone()))?;
                    collectors.insert(cache_key.clone(), CustomCollector::Counter(counter.clone()));
                    counter
                }
            };
            collector
                .with_label_values(&label_values)
                .inc_by(metric.value.max(0.0));
        }
        CustomMetricType::Gauge => {
            let collector = match collectors.get(&cache_key) {
                Some(CustomCollector::Gauge(gauge)) => gauge.clone(),
                Some(_) => {
                    return Err(format!(
                        "custom metric `{}` is already registered with a different type",
                        metric.name
                    ));
                }
                None => {
                    let opts = prometheus::Opts::new(&normalized_name, CUSTOM_METRIC_HELP);
                    let gauge = prometheus::GaugeVec::new(opts, &label_key_refs)
                        .map_err(|error| error.to_string())?;
                    register_collector(Box::new(gauge.clone()))?;
                    collectors.insert(cache_key.clone(), CustomCollector::Gauge(gauge.clone()));
                    gauge
                }
            };
            collector.with_label_values(&label_values).set(metric.value);
        }
        CustomMetricType::Histogram => {
            let collector = match collectors.get(&cache_key) {
                Some(CustomCollector::Histogram(histogram)) => histogram.clone(),
                Some(_) => {
                    return Err(format!(
                        "custom metric `{}` is already registered with a different type",
                        metric.name
                    ));
                }
                None => {
                    let opts = prometheus::HistogramOpts::new(&normalized_name, CUSTOM_METRIC_HELP);
                    let histogram = prometheus::HistogramVec::new(opts, &label_key_refs)
                        .map_err(|error| error.to_string())?;
                    register_collector(Box::new(histogram.clone()))?;
                    collectors.insert(
                        cache_key.clone(),
                        CustomCollector::Histogram(histogram.clone()),
                    );
                    histogram
                }
            };
            collector
                .with_label_values(&label_values)
                .observe(metric.value);
        }
    }
    drop(collectors);

    recover_poisoned(
        "custom_metrics",
        CUSTOM_METRICS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock(),
    )
    .insert(metric.name, metric.value);
    Ok(())
}

pub(crate) fn custom_metric_value(name: &str) -> Option<f64> {
    recover_poisoned(
        "custom_metrics",
        CUSTOM_METRICS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock(),
    )
    .get(name)
    .copied()
}

fn register_collector(collector: Box<dyn prometheus::core::Collector>) -> Result<(), String> {
    prometheus::default_registry()
        .register(collector)
        .map_err(|error| error.to_string())
}

fn collector_key(metric_type: CustomMetricType, name: &str, label_keys: &[String]) -> String {
    format!("{metric_type:?}:{name}:{}", label_keys.join(","))
}

fn normalize_metric_name(name: &str) -> Result<String, String> {
    let normalized = name
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == ':' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    validate_prometheus_ident(&normalized, "metric name")?;
    Ok(normalized)
}

fn normalize_label_name(name: &str) -> Result<String, String> {
    let normalized = name
        .trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    validate_prometheus_ident(&normalized, "label name")?;
    Ok(normalized)
}

fn validate_prometheus_ident(value: &str, kind: &str) -> Result<(), String> {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(format!("{kind} must not be empty"));
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == ':') {
        return Err(format!(
            "{kind} `{value}` must start with a letter, '_' or ':'"
        ));
    }
    if chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == ':') {
        Ok(())
    } else {
        Err(format!(
            "{kind} `{value}` may only contain ASCII letters, digits, '_' or ':'"
        ))
    }
}

pub(crate) fn init_telemetry_with_emitter<F>(emitter: F) -> TelemetryHandle
where
    F: Fn(String) -> bool + Send + Sync + 'static,
{
    let (sender, receiver) = mpsc::channel(TELEMETRY_CHANNEL_CAPACITY);
    let emitter: TelemetryEmitter = Arc::new(emitter);
    let snapshot = TelemetrySnapshotStore::default();

    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(run_telemetry_worker(
            receiver,
            Arc::clone(&emitter),
            snapshot.clone(),
        ));
    } else {
        let emitter = Arc::clone(&emitter);
        let snapshot_for_worker = snapshot.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test telemetry worker runtime should initialize");
            runtime.block_on(run_telemetry_worker(receiver, emitter, snapshot_for_worker));
        });
    }

    TelemetryHandle { sender, snapshot }
}

async fn run_telemetry_worker(
    mut receiver: mpsc::Receiver<TelemetryEvent>,
    emitter: TelemetryEmitter,
    snapshot: TelemetrySnapshotStore,
) {
    let mut requests = HashMap::new();

    while let Some(event) = receiver.recv().await {
        if let Some(completed) = apply_event(&mut requests, &snapshot, event) {
            if completed.sampled && !(emitter)(completed.line) {
                snapshot.record_dropped_event();
            }
        }
    }
}

fn apply_event(
    requests: &mut HashMap<String, RequestState>,
    snapshot: &TelemetrySnapshotStore,
    event: TelemetryEvent,
) -> Option<CompletedRequest> {
    match event {
        TelemetryEvent::RequestStart {
            trace_id,
            path,
            sampled,
            traceparent,
            timestamp,
        } => {
            snapshot.record_request_started();
            let state = requests.entry(trace_id).or_default();
            state.path = Some(path);
            state.sampled = sampled;
            state.traceparent = traceparent;
            state.request_started_at = Some(timestamp);
            None
        }
        TelemetryEvent::WasmStart {
            trace_id,
            timestamp,
        } => {
            requests.entry(trace_id).or_default().wasm_started_at = Some(timestamp);
            None
        }
        TelemetryEvent::WasmEnd {
            trace_id,
            timestamp,
        } => {
            requests.entry(trace_id).or_default().wasm_finished_at = Some(timestamp);
            None
        }
        TelemetryEvent::RequestEnd {
            trace_id,
            status,
            fuel_consumed,
            timestamp,
        } => {
            let mut state = requests.remove(&trace_id).unwrap_or_default();
            state.fuel_consumed = fuel_consumed;
            let completed = state.complete(trace_id, status, timestamp);
            snapshot.record_request_completed(&completed);
            Some(completed)
        }
    }
}

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        let _ = self
            .active_requests
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                current.checked_sub(1)
            });
    }
}

impl Default for TelemetrySnapshotStore {
    fn default() -> Self {
        Self {
            metrics: Arc::new(Mutex::new(AggregatedMetrics::default())),
            active_requests: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl TelemetrySnapshotStore {
    fn record_request_started(&self) {
        let mut metrics = recover_poisoned("telemetry_snapshot", self.metrics.lock());
        metrics.total_requests = metrics.total_requests.saturating_add(1);
    }

    fn record_request_completed(&self, completed: &CompletedRequest) {
        let mut metrics = recover_poisoned("telemetry_snapshot", self.metrics.lock());

        metrics.completed_requests = metrics.completed_requests.saturating_add(1);
        metrics.last_status = completed.status;
        if completed.status >= ERROR_STATUS_THRESHOLD {
            metrics.error_requests = metrics.error_requests.saturating_add(1);
        }
        metrics.total_duration_us = metrics
            .total_duration_us
            .saturating_add(completed.total_duration_us);
        metrics.total_wasm_duration_us = metrics
            .total_wasm_duration_us
            .saturating_add(completed.wasm_duration_us);
        metrics.total_host_overhead_us = metrics
            .total_host_overhead_us
            .saturating_add(completed.host_overhead_us);
    }

    fn record_dropped_event(&self) {
        let mut metrics = recover_poisoned("telemetry_snapshot", self.metrics.lock());
        metrics.dropped_events = metrics.dropped_events.saturating_add(1);
    }

    fn snapshot(&self) -> TelemetrySnapshot {
        let metrics = recover_poisoned("telemetry_snapshot", self.metrics.lock());

        TelemetrySnapshot {
            total_requests: metrics.total_requests,
            completed_requests: metrics.completed_requests,
            error_requests: metrics.error_requests,
            active_requests: self
                .active_requests
                .load(Ordering::Relaxed)
                .min(u32::MAX as usize) as u32,
            dropped_events: metrics.dropped_events,
            last_status: metrics.last_status,
            total_duration_us: metrics.total_duration_us,
            total_wasm_duration_us: metrics.total_wasm_duration_us,
            total_host_overhead_us: metrics.total_host_overhead_us,
        }
    }
}

impl RequestState {
    fn complete(self, trace_id: String, status: u16, completed_at: Instant) -> CompletedRequest {
        let total_duration_us = duration_us(self.request_started_at, completed_at);
        let wasm_duration_us = match (self.wasm_started_at, self.wasm_finished_at) {
            (Some(started_at), Some(finished_at)) => u128_to_u64(
                finished_at
                    .saturating_duration_since(started_at)
                    .as_micros(),
            ),
            _ => 0,
        };
        let host_overhead_us = total_duration_us.saturating_sub(wasm_duration_us);

        CompletedRequest {
            line: json!({
                "trace_id": trace_id,
                "sampled": self.sampled,
                "traceparent": self.traceparent,
                "path": self.path,
                "status": status,
                "fuel_consumed": self.fuel_consumed,
                "total_duration_us": total_duration_us,
                "wasm_duration_us": wasm_duration_us,
                "host_overhead_us": host_overhead_us,
            })
            .to_string(),
            sampled: self.sampled,
            status,
            total_duration_us,
            wasm_duration_us,
            host_overhead_us,
        }
    }
}

fn duration_us(started_at: Option<Instant>, completed_at: Instant) -> u64 {
    started_at
        .map(|started_at| {
            u128_to_u64(
                completed_at
                    .saturating_duration_since(started_at)
                    .as_micros(),
            )
        })
        .unwrap_or_default()
}

fn u128_to_u64(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
pub(crate) fn init_test_telemetry() -> TelemetryHandle {
    init_telemetry_with_emitter(|_| true)
}

#[cfg(test)]
pub(crate) fn init_test_telemetry_with_emitter<F>(emitter: F) -> TelemetryHandle
where
    F: Fn(String) -> bool + Send + Sync + 'static,
{
    init_telemetry_with_emitter(emitter)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::time::Duration;

    #[test]
    fn request_completion_calculates_overhead_metrics() {
        let started_at = Instant::now();
        let mut requests = HashMap::new();
        let snapshot = TelemetrySnapshotStore::default();

        apply_event(
            &mut requests,
            &snapshot,
            TelemetryEvent::RequestStart {
                trace_id: "trace-1".to_owned(),
                path: "/api/guest-example".to_owned(),
                sampled: true,
                traceparent: Some(
                    "00-00000000000000000000000000000001-0000000000000001-01".to_owned(),
                ),
                timestamp: started_at,
            },
        );
        apply_event(
            &mut requests,
            &snapshot,
            TelemetryEvent::WasmStart {
                trace_id: "trace-1".to_owned(),
                timestamp: started_at + Duration::from_micros(10),
            },
        );
        apply_event(
            &mut requests,
            &snapshot,
            TelemetryEvent::WasmEnd {
                trace_id: "trace-1".to_owned(),
                timestamp: started_at + Duration::from_micros(60),
            },
        );
        let completed = apply_event(
            &mut requests,
            &snapshot,
            TelemetryEvent::RequestEnd {
                trace_id: "trace-1".to_owned(),
                status: 200,
                fuel_consumed: Some(42),
                timestamp: started_at + Duration::from_micros(100),
            },
        )
        .expect("completed request should emit a metrics record");

        let record: Value =
            serde_json::from_str(&completed.line).expect("telemetry worker should emit valid JSON");

        assert_eq!(record["trace_id"], "trace-1");
        assert_eq!(record["sampled"], true);
        assert_eq!(
            record["traceparent"],
            "00-00000000000000000000000000000001-0000000000000001-01"
        );
        assert_eq!(record["path"], "/api/guest-example");
        assert_eq!(record["status"], 200);
        assert_eq!(record["fuel_consumed"], 42);
        assert_eq!(record["total_duration_us"], 100);
        assert_eq!(record["wasm_duration_us"], 50);
        assert_eq!(record["host_overhead_us"], 50);
        assert!(requests.is_empty());

        let snapshot = snapshot.snapshot();
        assert_eq!(snapshot.total_requests, 1);
        assert_eq!(snapshot.completed_requests, 1);
        assert_eq!(snapshot.error_requests, 0);
        assert_eq!(snapshot.total_duration_us, 100);
        assert_eq!(snapshot.total_wasm_duration_us, 50);
        assert_eq!(snapshot.total_host_overhead_us, 50);
    }

    #[test]
    fn request_completion_without_wasm_events_reports_zero_wasm_duration() {
        let started_at = Instant::now();
        let mut requests = HashMap::new();
        let snapshot = TelemetrySnapshotStore::default();

        apply_event(
            &mut requests,
            &snapshot,
            TelemetryEvent::RequestStart {
                trace_id: "trace-404".to_owned(),
                path: "/api/missing".to_owned(),
                sampled: false,
                traceparent: None,
                timestamp: started_at,
            },
        );
        let completed = apply_event(
            &mut requests,
            &snapshot,
            TelemetryEvent::RequestEnd {
                trace_id: "trace-404".to_owned(),
                status: 404,
                fuel_consumed: None,
                timestamp: started_at + Duration::from_micros(25),
            },
        )
        .expect("request end should still emit a metrics record");

        let record: Value =
            serde_json::from_str(&completed.line).expect("telemetry worker should emit valid JSON");

        assert_eq!(record["status"], 404);
        assert_eq!(record["total_duration_us"], 25);
        assert_eq!(record["wasm_duration_us"], 0);
        assert_eq!(record["host_overhead_us"], 25);
        assert!(requests.is_empty());

        let snapshot = snapshot.snapshot();
        assert_eq!(snapshot.error_requests, 1);
        assert_eq!(snapshot.last_status, 404);
    }

    #[test]
    fn active_request_guard_tracks_current_pressure() {
        let telemetry = init_test_telemetry();

        {
            let _guard = begin_request(&telemetry);
            assert_eq!(active_requests(&telemetry), 1);
        }

        assert_eq!(active_requests(&telemetry), 0);
    }

    #[test]
    fn custom_metric_push_tracks_latest_value() {
        push_custom_metric(CustomMetric {
            name: "checkout.conversion".to_owned(),
            value: 0.12,
            metric_type: CustomMetricType::Gauge,
            tags: vec![("route".to_owned(), "/shop".to_owned())],
        })
        .expect("custom gauge should be accepted");

        assert_eq!(custom_metric_value("checkout.conversion"), Some(0.12));
    }

    #[test]
    fn invalid_metric_names_are_rejected() {
        let error = push_custom_metric(CustomMetric {
            name: "123-invalid".to_owned(),
            value: 1.0,
            metric_type: CustomMetricType::Counter,
            tags: vec![],
        })
        .expect_err("invalid metric name should fail");

        assert!(error.contains("metric name"));
    }
}
