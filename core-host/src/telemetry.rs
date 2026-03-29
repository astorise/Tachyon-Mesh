use serde_json::json;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Instant,
};
use tokio::sync::mpsc;

const TELEMETRY_CHANNEL_CAPACITY: usize = 10_000;
const ERROR_STATUS_THRESHOLD: u16 = 400;

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
        timestamp: Instant,
    },
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
    request_started_at: Option<Instant>,
    wasm_started_at: Option<Instant>,
    wasm_finished_at: Option<Instant>,
}

struct CompletedRequest {
    line: String,
    status: u16,
    total_duration_us: u64,
    wasm_duration_us: u64,
    host_overhead_us: u64,
}

type TelemetryEmitter = Arc<dyn Fn(String) + Send + Sync>;

pub(crate) struct ActiveRequestGuard {
    active_requests: Arc<AtomicUsize>,
}

pub(crate) fn init_telemetry() -> TelemetryHandle {
    init_telemetry_with_emitter(|line| println!("{line}"))
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

fn init_telemetry_with_emitter<F>(emitter: F) -> TelemetryHandle
where
    F: Fn(String) + Send + Sync + 'static,
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
            emitter(completed.line);
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
            timestamp,
        } => {
            snapshot.record_request_started();
            let state = requests.entry(trace_id).or_default();
            state.path = Some(path);
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
            timestamp,
        } => {
            let state = requests.remove(&trace_id).unwrap_or_default();
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
        let mut metrics = self
            .metrics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        metrics.total_requests = metrics.total_requests.saturating_add(1);
    }

    fn record_request_completed(&self, completed: &CompletedRequest) {
        let mut metrics = self
            .metrics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

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
        let mut metrics = self
            .metrics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        metrics.dropped_events = metrics.dropped_events.saturating_add(1);
    }

    fn snapshot(&self) -> TelemetrySnapshot {
        let metrics = self
            .metrics
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

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
                "path": self.path,
                "status": status,
                "total_duration_us": total_duration_us,
                "wasm_duration_us": wasm_duration_us,
                "host_overhead_us": host_overhead_us,
            })
            .to_string(),
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
    init_telemetry_with_emitter(|_| {})
}

#[cfg(test)]
pub(crate) fn init_test_telemetry_with_emitter<F>(emitter: F) -> TelemetryHandle
where
    F: Fn(String) + Send + Sync + 'static,
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
                timestamp: started_at + Duration::from_micros(100),
            },
        )
        .expect("completed request should emit a metrics record");

        let record: Value =
            serde_json::from_str(&completed.line).expect("telemetry worker should emit valid JSON");

        assert_eq!(record["trace_id"], "trace-1");
        assert_eq!(record["path"], "/api/guest-example");
        assert_eq!(record["status"], 200);
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
                timestamp: started_at,
            },
        );
        let completed = apply_event(
            &mut requests,
            &snapshot,
            TelemetryEvent::RequestEnd {
                trace_id: "trace-404".to_owned(),
                status: 404,
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
}
