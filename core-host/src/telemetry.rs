use serde_json::json;
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::sync::mpsc;

const TELEMETRY_CHANNEL_CAPACITY: usize = 10_000;

pub(crate) type TelemetrySender = mpsc::Sender<TelemetryEvent>;

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
struct RequestState {
    path: Option<String>,
    request_started_at: Option<Instant>,
    wasm_started_at: Option<Instant>,
    wasm_finished_at: Option<Instant>,
}

type TelemetryEmitter = Arc<dyn Fn(String) + Send + Sync>;

pub(crate) fn init_telemetry() -> TelemetrySender {
    init_telemetry_with_emitter(|line| println!("{line}"))
}

pub(crate) fn record_event(sender: &TelemetrySender, event: TelemetryEvent) {
    let _ = sender.try_send(event);
}

fn init_telemetry_with_emitter<F>(emitter: F) -> TelemetrySender
where
    F: Fn(String) + Send + Sync + 'static,
{
    let (sender, receiver) = mpsc::channel(TELEMETRY_CHANNEL_CAPACITY);
    let emitter: TelemetryEmitter = Arc::new(emitter);

    tokio::spawn(run_telemetry_worker(receiver, emitter));
    sender
}

async fn run_telemetry_worker(
    mut receiver: mpsc::Receiver<TelemetryEvent>,
    emitter: TelemetryEmitter,
) {
    let mut requests = HashMap::new();

    while let Some(event) = receiver.recv().await {
        if let Some(line) = apply_event(&mut requests, event) {
            emitter(line);
        }
    }
}

fn apply_event(
    requests: &mut HashMap<String, RequestState>,
    event: TelemetryEvent,
) -> Option<String> {
    match event {
        TelemetryEvent::RequestStart {
            trace_id,
            path,
            timestamp,
        } => {
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
            Some(state.complete(trace_id, status, timestamp))
        }
    }
}

impl RequestState {
    fn complete(self, trace_id: String, status: u16, completed_at: Instant) -> String {
        let total_duration_us = duration_us(self.request_started_at, completed_at);
        let wasm_duration_us = match (self.wasm_started_at, self.wasm_finished_at) {
            (Some(started_at), Some(finished_at)) => finished_at
                .saturating_duration_since(started_at)
                .as_micros(),
            _ => 0,
        };
        let host_overhead_us = total_duration_us.saturating_sub(wasm_duration_us);

        json!({
            "trace_id": trace_id,
            "path": self.path,
            "status": status,
            "total_duration_us": total_duration_us,
            "wasm_duration_us": wasm_duration_us,
            "host_overhead_us": host_overhead_us,
        })
        .to_string()
    }
}

fn duration_us(started_at: Option<Instant>, completed_at: Instant) -> u128 {
    started_at
        .map(|started_at| {
            completed_at
                .saturating_duration_since(started_at)
                .as_micros()
        })
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) fn init_test_telemetry() -> TelemetrySender {
    init_telemetry_with_emitter(|_| {})
}

#[cfg(test)]
pub(crate) fn init_test_telemetry_with_emitter<F>(emitter: F) -> TelemetrySender
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

        apply_event(
            &mut requests,
            TelemetryEvent::RequestStart {
                trace_id: "trace-1".to_owned(),
                path: "/api/guest-example".to_owned(),
                timestamp: started_at,
            },
        );
        apply_event(
            &mut requests,
            TelemetryEvent::WasmStart {
                trace_id: "trace-1".to_owned(),
                timestamp: started_at + Duration::from_micros(10),
            },
        );
        apply_event(
            &mut requests,
            TelemetryEvent::WasmEnd {
                trace_id: "trace-1".to_owned(),
                timestamp: started_at + Duration::from_micros(60),
            },
        );
        let line = apply_event(
            &mut requests,
            TelemetryEvent::RequestEnd {
                trace_id: "trace-1".to_owned(),
                status: 200,
                timestamp: started_at + Duration::from_micros(100),
            },
        )
        .expect("completed request should emit a metrics record");

        let record: Value =
            serde_json::from_str(&line).expect("telemetry worker should emit valid JSON");

        assert_eq!(record["trace_id"], "trace-1");
        assert_eq!(record["path"], "/api/guest-example");
        assert_eq!(record["status"], 200);
        assert_eq!(record["total_duration_us"], 100);
        assert_eq!(record["wasm_duration_us"], 50);
        assert_eq!(record["host_overhead_us"], 50);
        assert!(requests.is_empty());
    }

    #[test]
    fn request_completion_without_wasm_events_reports_zero_wasm_duration() {
        let started_at = Instant::now();
        let mut requests = HashMap::new();

        apply_event(
            &mut requests,
            TelemetryEvent::RequestStart {
                trace_id: "trace-404".to_owned(),
                path: "/api/missing".to_owned(),
                timestamp: started_at,
            },
        );
        let line = apply_event(
            &mut requests,
            TelemetryEvent::RequestEnd {
                trace_id: "trace-404".to_owned(),
                status: 404,
                timestamp: started_at + Duration::from_micros(25),
            },
        )
        .expect("request end should still emit a metrics record");

        let record: Value =
            serde_json::from_str(&line).expect("telemetry worker should emit valid JSON");

        assert_eq!(record["status"], 404);
        assert_eq!(record["total_duration_us"], 25);
        assert_eq!(record["wasm_duration_us"], 0);
        assert_eq!(record["host_overhead_us"], 25);
        assert!(requests.is_empty());
    }
}
