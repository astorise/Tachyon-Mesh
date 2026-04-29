use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "control-plane-faas",
    });

    export!(Component);
}

const NODE_ID_ENV: &str = "NODE_ID";
const LIMIT_ENV: &str = "DIST_LIMIT";
const WINDOW_SECONDS_ENV: &str = "DIST_LIMIT_WINDOW_SECONDS";
const DEFAULT_LIMIT: u64 = 100;
const DEFAULT_WINDOW_SECONDS: u64 = 60;

static COUNTERS: OnceLock<Mutex<GCounterSet>> = OnceLock::new();

struct Component;

impl bindings::Guest for Component {
    fn on_tick() {}
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        match (req.method.as_str(), request_path(&req.uri).as_str()) {
            ("POST", "/check") => check(req.body),
            ("POST", "/merge") => merge(req.body),
            ("GET", "/state") => {
                json_response(200, counters().lock().expect("counter state").snapshot())
            }
            _ => response(404, b"unknown distributed limiter endpoint".to_vec()),
        }
    }
}

fn check(body: Vec<u8>) -> bindings::exports::tachyon::mesh::handler::Response {
    let request = match serde_json::from_slice::<CheckRequest>(&body) {
        Ok(request) => request,
        Err(error) => {
            return response(
                400,
                format!("invalid limiter request: {error}").into_bytes(),
            )
        }
    };
    let node_id = env_or_default(NODE_ID_ENV, "local");
    let window = current_window(parse_u64_env(WINDOW_SECONDS_ENV, DEFAULT_WINDOW_SECONDS));
    let total = counters()
        .lock()
        .expect("counter state")
        .increment(&request.key, window, &node_id);
    json_response(
        200,
        &CheckResponse {
            allowed: total <= parse_u64_env(LIMIT_ENV, DEFAULT_LIMIT),
            total,
        },
    )
}

fn merge(body: Vec<u8>) -> bindings::exports::tachyon::mesh::handler::Response {
    let remote = match serde_json::from_slice::<CounterSnapshot>(&body) {
        Ok(remote) => remote,
        Err(error) => return response(400, format!("invalid CRDT state: {error}").into_bytes()),
    };
    counters().lock().expect("counter state").merge(remote);
    response(204, Vec::new())
}

fn counters() -> &'static Mutex<GCounterSet> {
    COUNTERS.get_or_init(|| Mutex::new(GCounterSet::default()))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GCounterSet {
    counters: HashMap<String, HashMap<String, u64>>,
}

impl GCounterSet {
    pub fn increment(&mut self, key: &str, window: u64, node_id: &str) -> u64 {
        let key = format!("{key}:{window}");
        let node_counts = self.counters.entry(key).or_default();
        *node_counts.entry(node_id.to_owned()).or_default() += 1;
        node_counts.values().sum()
    }

    pub fn merge(&mut self, remote: CounterSnapshot) {
        for (key, remote_nodes) in remote.counters {
            let local_nodes = self.counters.entry(key).or_default();
            for (node, remote_value) in remote_nodes {
                let local_value = local_nodes.entry(node).or_default();
                *local_value = (*local_value).max(remote_value);
            }
        }
    }

    pub fn snapshot(&self) -> CounterSnapshot {
        CounterSnapshot {
            counters: self.counters.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CounterSnapshot {
    counters: HashMap<String, HashMap<String, u64>>,
}

#[derive(Deserialize)]
struct CheckRequest {
    key: String,
}

#[derive(Serialize)]
struct CheckResponse {
    allowed: bool,
    total: u64,
}

fn current_window(window_seconds: u64) -> u64 {
    now_seconds() / window_seconds.max(1)
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn parse_u64_env(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn env_or_default(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn request_path(uri: &str) -> String {
    let path = uri.split('?').next().unwrap_or(uri).trim();
    if path.is_empty() || path == "/" {
        "/".to_owned()
    } else if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    }
}

fn json_response<T: Serialize>(
    status: u16,
    value: T,
) -> bindings::exports::tachyon::mesh::handler::Response {
    match serde_json::to_vec(&value) {
        Ok(body) => bindings::exports::tachyon::mesh::handler::Response {
            status,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body,
            trailers: Vec::new(),
        },
        Err(error) => response(
            500,
            format!("failed to encode limiter response: {error}").into_bytes(),
        ),
    }
}

fn response(status: u16, body: Vec<u8>) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: Vec::new(),
        body,
        trailers: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::GCounterSet;

    #[test]
    fn gcounter_merge_converges_by_taking_node_maxima() {
        let mut left = GCounterSet::default();
        left.increment("203.0.113.10", 42, "node-a");
        left.increment("203.0.113.10", 42, "node-a");

        let mut right = GCounterSet::default();
        right.increment("203.0.113.10", 42, "node-b");
        right.merge(left.snapshot());
        left.merge(right.snapshot());

        assert_eq!(
            left.snapshot()
                .counters
                .get("203.0.113.10:42")
                .expect("counter should exist")
                .values()
                .sum::<u64>(),
            3
        );
    }
}
