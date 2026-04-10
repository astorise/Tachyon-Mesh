mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

const SESSION_DIR: &str = "/sessions";
const PEER_URLS_ENV: &str = "PEER_URLS";
const GOSSIP_PATH_ENV: &str = "GOSSIP_PATH";
const BRIDGE_SOFT_LIMIT_ENV: &str = "BRIDGE_SOFT_LIMIT";
const DEFAULT_GOSSIP_PATH: &str = "/system/gossip";
const DEFAULT_BRIDGE_SOFT_LIMIT: u8 = 80;
const DELEGATED_HEADER: &str = "x-tachyon-bridge-delegated";
const SYSTEM_BRIDGE_ROUTE: &str = "/system/bridge";

static PICK_COUNTER: AtomicU64 = AtomicU64::new(0);

struct Component;

#[derive(Debug, Deserialize)]
struct CreateBridgeRequest {
    client_a_addr: String,
    client_b_addr: String,
    #[serde(default = "default_timeout_seconds")]
    timeout_seconds: u32,
}

#[derive(Debug, Deserialize, Serialize)]
struct DestroyBridgeRequest {
    bridge_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct BridgeResponse {
    bridge_id: String,
    ip: String,
    port_a: u16,
    port_b: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct BridgeSessionRecord {
    bridge_id: String,
    ip: String,
    client_a_addr: String,
    client_b_addr: String,
    timeout_seconds: u32,
    port_a: u16,
    port_b: u16,
    status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    delegate_base_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct TelemetrySnapshot {
    #[serde(default)]
    active_requests: u32,
    #[serde(default)]
    active_l4_relays: u32,
    #[serde(default)]
    l4_load_score: u8,
    #[serde(default)]
    advertise_ip: String,
}

#[derive(Clone, Debug)]
struct PeerEndpoint {
    base_url: String,
    snapshot_url: String,
}

#[derive(Clone, Debug)]
struct PeerCandidate {
    base_url: String,
    snapshot: TelemetrySnapshot,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        match req.method.as_str() {
            "POST" => create_bridge(req),
            "DELETE" => destroy_bridge(req.body),
            _ => response(405, "unsupported method"),
        }
    }
}

fn create_bridge(
    req: bindings::exports::tachyon::mesh::handler::Request,
) -> bindings::exports::tachyon::mesh::handler::Response {
    let request = match serde_json::from_slice::<CreateBridgeRequest>(&req.body) {
        Ok(request) => request,
        Err(error) => return response(400, format!("invalid create bridge payload: {error}")),
    };

    if !has_header(&req.headers, DELEGATED_HEADER) {
        if let Some(peer) = choose_delegate_peer() {
            match delegate_bridge_create(&peer.base_url, &req.body) {
                Ok(bridge) => {
                    let session = build_session_record(
                        &request,
                        &bridge,
                        "delegated",
                        Some(peer.base_url.clone()),
                    );
                    if let Err(error) = persist_session(&session) {
                        return response(
                            500,
                            format!("failed to persist delegated bridge session: {error}"),
                        );
                    }
                    return json_response(200, &bridge);
                }
                Err(error) => {
                    eprintln!("system-faas-bridge failed to delegate bridge allocation: {error}");
                }
            }
        }
    }

    let bridge = match allocate_local_bridge(&request) {
        Ok(bridge) => bridge,
        Err(error) => return response(502, format!("bridge allocation failed: {error}")),
    };

    let session = build_session_record(&request, &bridge, "active", None);
    if let Err(error) = persist_session(&session) {
        return response(500, format!("failed to persist bridge session: {error}"));
    }

    json_response(200, &bridge)
}

fn destroy_bridge(body: Vec<u8>) -> bindings::exports::tachyon::mesh::handler::Response {
    let request = match serde_json::from_slice::<DestroyBridgeRequest>(&body) {
        Ok(request) => request,
        Err(error) => return response(400, format!("invalid destroy bridge payload: {error}")),
    };

    let existing = load_session(&request.bridge_id).ok();
    let destroy_result = if let Some(delegate_base_url) = existing
        .as_ref()
        .and_then(|session| session.delegate_base_url.as_deref())
    {
        delegate_bridge_destroy(delegate_base_url, &request.bridge_id)
    } else {
        bindings::tachyon::mesh::bridge_controller::destroy_bridge(&request.bridge_id)
    };
    if let Err(error) = destroy_result {
        return response(502, format!("bridge teardown failed: {error}"));
    }

    let previous = existing.unwrap_or_else(|| BridgeSessionRecord {
        bridge_id: request.bridge_id.clone(),
        ip: String::new(),
        client_a_addr: String::new(),
        client_b_addr: String::new(),
        timeout_seconds: 0,
        port_a: 0,
        port_b: 0,
        status: "active".to_owned(),
        delegate_base_url: None,
    });
    let session = BridgeSessionRecord {
        bridge_id: request.bridge_id,
        ip: previous.ip,
        client_a_addr: previous.client_a_addr,
        client_b_addr: previous.client_b_addr,
        timeout_seconds: previous.timeout_seconds,
        port_a: previous.port_a,
        port_b: previous.port_b,
        status: "destroyed".to_owned(),
        delegate_base_url: previous.delegate_base_url,
    };
    if let Err(error) = persist_session(&session) {
        return response(500, format!("failed to persist bridge teardown: {error}"));
    }

    response(204, Vec::<u8>::new())
}

fn allocate_local_bridge(request: &CreateBridgeRequest) -> Result<BridgeResponse, String> {
    let allocation = bindings::tachyon::mesh::bridge_controller::create_bridge(
        &bindings::tachyon::mesh::bridge_controller::BridgeConfig {
            client_a_addr: request.client_a_addr.clone(),
            client_b_addr: request.client_b_addr.clone(),
            timeout_seconds: request.timeout_seconds,
        },
    )?;

    Ok(BridgeResponse {
        bridge_id: allocation.bridge_id,
        ip: allocation.ip,
        port_a: allocation.port_a,
        port_b: allocation.port_b,
    })
}

fn build_session_record(
    request: &CreateBridgeRequest,
    bridge: &BridgeResponse,
    status: &str,
    delegate_base_url: Option<String>,
) -> BridgeSessionRecord {
    BridgeSessionRecord {
        bridge_id: bridge.bridge_id.clone(),
        ip: bridge.ip.clone(),
        client_a_addr: request.client_a_addr.clone(),
        client_b_addr: request.client_b_addr.clone(),
        timeout_seconds: request.timeout_seconds,
        port_a: bridge.port_a,
        port_b: bridge.port_b,
        status: status.to_owned(),
        delegate_base_url,
    }
}

fn choose_delegate_peer() -> Option<PeerCandidate> {
    let local = local_snapshot();
    let soft_limit = parse_limit(BRIDGE_SOFT_LIMIT_ENV, DEFAULT_BRIDGE_SOFT_LIMIT).ok()?;
    if local.l4_load_score < soft_limit {
        return None;
    }

    let peers = fetch_peer_snapshots().ok()?;
    select_delegate_peer(local.l4_load_score, &peers)
}

fn fetch_peer_snapshots() -> Result<Vec<PeerCandidate>, String> {
    let gossip_path = env_or_default(GOSSIP_PATH_ENV, DEFAULT_GOSSIP_PATH);
    let peers = parse_peer_entries(
        &std::env::var(PEER_URLS_ENV).unwrap_or_default(),
        &gossip_path,
    );
    let mut snapshots = Vec::new();
    for peer in peers {
        let response = bindings::tachyon::mesh::outbound_http::send_request(
            "GET",
            &peer.snapshot_url,
            &[],
            &[],
        )?;
        if response.status >= 400 {
            continue;
        }
        if let Ok(snapshot) = serde_json::from_slice::<TelemetrySnapshot>(&response.body) {
            snapshots.push(PeerCandidate {
                base_url: peer.base_url,
                snapshot,
            });
        }
    }
    Ok(snapshots)
}

fn local_snapshot() -> TelemetrySnapshot {
    let snapshot = bindings::tachyon::mesh::telemetry_reader::get_metrics();
    TelemetrySnapshot {
        active_requests: snapshot.active_requests,
        active_l4_relays: snapshot.active_l4_relays,
        l4_load_score: snapshot.l4_load_score,
        advertise_ip: snapshot.advertise_ip,
    }
}

fn delegate_bridge_create(base_url: &str, body: &[u8]) -> Result<BridgeResponse, String> {
    let response = bindings::tachyon::mesh::outbound_http::send_request(
        "POST",
        &format!("{}{SYSTEM_BRIDGE_ROUTE}", base_url.trim_end_matches('/')),
        &[
            ("content-type".to_owned(), "application/json".to_owned()),
            (DELEGATED_HEADER.to_owned(), "true".to_owned()),
        ],
        body,
    )?;
    if response.status >= 400 {
        return Err(format!(
            "peer bridge controller returned HTTP {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }
    serde_json::from_slice(&response.body)
        .map_err(|error| format!("failed to decode delegated bridge response: {error}"))
}

fn delegate_bridge_destroy(base_url: &str, bridge_id: &str) -> Result<(), String> {
    let body = serde_json::to_vec(&DestroyBridgeRequest {
        bridge_id: bridge_id.to_owned(),
    })
    .map_err(|error| format!("failed to encode bridge teardown payload: {error}"))?;
    let response = bindings::tachyon::mesh::outbound_http::send_request(
        "DELETE",
        &format!("{}{SYSTEM_BRIDGE_ROUTE}", base_url.trim_end_matches('/')),
        &[
            ("content-type".to_owned(), "application/json".to_owned()),
            (DELEGATED_HEADER.to_owned(), "true".to_owned()),
        ],
        &body,
    )?;
    if response.status >= 400 {
        return Err(format!(
            "peer bridge teardown returned HTTP {}: {}",
            response.status,
            String::from_utf8_lossy(&response.body)
        ));
    }
    Ok(())
}

fn load_session(bridge_id: &str) -> Result<BridgeSessionRecord, String> {
    let path = session_path(bridge_id);
    let payload = std::fs::read(path)
        .map_err(|error| format!("failed to read bridge session file: {error}"))?;
    serde_json::from_slice(&payload)
        .map_err(|error| format!("failed to decode bridge session record: {error}"))
}

fn persist_session(session: &BridgeSessionRecord) -> Result<(), String> {
    std::fs::create_dir_all(SESSION_DIR)
        .map_err(|error| format!("failed to create session dir: {error}"))?;
    let path = session_path(&session.bridge_id);
    let payload = serde_json::to_vec(session)
        .map_err(|error| format!("failed to serialize session: {error}"))?;
    std::fs::write(path, payload).map_err(|error| format!("failed to write session file: {error}"))
}

fn session_path(bridge_id: &str) -> String {
    format!("{SESSION_DIR}/{bridge_id}.json")
}

fn default_timeout_seconds() -> u32 {
    30
}

fn response(
    status: u16,
    body: impl Into<Vec<u8>>,
) -> bindings::exports::tachyon::mesh::handler::Response {
    response_with_headers(status, body, &[])
}

fn json_response<T: Serialize>(
    status: u16,
    body: &T,
) -> bindings::exports::tachyon::mesh::handler::Response {
    match serde_json::to_vec(body) {
        Ok(body) => response_with_headers(status, body, &[("content-type", "application/json")]),
        Err(error) => response(500, format!("failed to encode bridge response: {error}")),
    }
}

fn response_with_headers(
    status: u16,
    body: impl Into<Vec<u8>>,
    headers: &[(&str, &str)],
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        body: body.into(),
        trailers: vec![],
    }
}

fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers
        .iter()
        .any(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
}

fn env_or_default(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn parse_limit(name: &str, default: u8) -> Result<u8, String> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => value
            .trim()
            .parse::<u8>()
            .map_err(|error| format!("failed to parse `{name}` as u8: {error}")),
        _ => Ok(default),
    }
}

fn parse_peer_entries(value: &str, gossip_path: &str) -> Vec<PeerEndpoint> {
    value
        .split(',')
        .filter_map(|entry| {
            let trimmed = entry.trim().trim_end_matches('/');
            if trimmed.is_empty() {
                return None;
            }

            let normalized_gossip = normalize_route(gossip_path).ok()?;
            if trimmed.ends_with(&normalized_gossip) {
                let base_url = trimmed
                    .strip_suffix(&normalized_gossip)
                    .unwrap_or(trimmed)
                    .trim_end_matches('/')
                    .to_owned();
                return Some(PeerEndpoint {
                    base_url: base_url.clone(),
                    snapshot_url: format!("{base_url}{normalized_gossip}"),
                });
            }

            Some(PeerEndpoint {
                base_url: trimmed.to_owned(),
                snapshot_url: format!("{trimmed}{normalized_gossip}"),
            })
        })
        .collect()
}

fn normalize_route(route: &str) -> Result<String, String> {
    let trimmed = route.trim();
    if trimmed.is_empty() {
        return Err("route must not be empty".to_owned());
    }
    Ok(if trimmed.starts_with('/') {
        trimmed.trim_end_matches('/').to_owned()
    } else {
        format!("/{}", trimmed.trim_end_matches('/'))
    })
}

fn select_delegate_peer(local_l4_load: u8, peers: &[PeerCandidate]) -> Option<PeerCandidate> {
    let mut candidates = peers
        .iter()
        .filter(|peer| peer.snapshot.l4_load_score < local_l4_load)
        .cloned()
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }
    if candidates.len() == 1 {
        return candidates.pop();
    }

    let seed = PICK_COUNTER.fetch_add(1, Ordering::Relaxed) as usize;
    let first = &candidates[seed % candidates.len()];
    let second = &candidates[(seed + 1) % candidates.len()];
    Some(prefer_peer(first, second).clone())
}

fn prefer_peer<'a>(left: &'a PeerCandidate, right: &'a PeerCandidate) -> &'a PeerCandidate {
    match left
        .snapshot
        .l4_load_score
        .cmp(&right.snapshot.l4_load_score)
        .then_with(|| {
            left.snapshot
                .active_l4_relays
                .cmp(&right.snapshot.active_l4_relays)
        })
        .then_with(|| {
            left.snapshot
                .active_requests
                .cmp(&right.snapshot.active_requests)
        }) {
        std::cmp::Ordering::Less => left,
        std::cmp::Ordering::Greater => right,
        std::cmp::Ordering::Equal => {
            if !left.snapshot.advertise_ip.is_empty() && right.snapshot.advertise_ip.is_empty() {
                left
            } else {
                right
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(base_url: &str, l4_load_score: u8, active_l4_relays: u32) -> PeerCandidate {
        PeerCandidate {
            base_url: base_url.to_owned(),
            snapshot: TelemetrySnapshot {
                active_requests: active_l4_relays,
                active_l4_relays,
                l4_load_score,
                advertise_ip: String::new(),
            },
        }
    }

    #[test]
    fn default_timeout_is_30_seconds() {
        assert_eq!(default_timeout_seconds(), 30);
    }

    #[test]
    fn bridge_response_serializes_json_shape() {
        let encoded = serde_json::to_value(BridgeResponse {
            bridge_id: "bridge-1".to_owned(),
            ip: "203.0.113.50".to_owned(),
            port_a: 10_000,
            port_b: 10_001,
        })
        .expect("bridge response should serialize");
        assert_eq!(encoded["bridge_id"], "bridge-1");
        assert_eq!(encoded["ip"], "203.0.113.50");
        assert_eq!(encoded["port_a"], 10_000);
        assert_eq!(encoded["port_b"], 10_001);
    }

    #[test]
    fn peer_entries_accept_base_urls_and_full_paths() {
        let peers = parse_peer_entries(
            "http://node-a:8080, http://node-b:8080/system/gossip",
            "/system/gossip",
        );

        assert_eq!(peers.len(), 2);
        assert_eq!(peers[0].base_url, "http://node-a:8080");
        assert_eq!(peers[0].snapshot_url, "http://node-a:8080/system/gossip");
        assert_eq!(peers[1].base_url, "http://node-b:8080");
        assert_eq!(peers[1].snapshot_url, "http://node-b:8080/system/gossip");
    }

    #[test]
    fn select_delegate_peer_prefers_lower_l4_load_score() {
        PICK_COUNTER.store(0, Ordering::Relaxed);
        let peers = vec![
            peer("http://node-a", 90, 4),
            peer("http://node-b", 10, 1),
            peer("http://node-c", 20, 2),
        ];

        let selected = select_delegate_peer(95, &peers).expect("a peer should be selected");
        assert_eq!(selected.base_url, "http://node-b");
    }

    #[test]
    fn select_delegate_peer_returns_none_when_no_peer_is_healthier() {
        let peers = vec![peer("http://node-a", 90, 4), peer("http://node-b", 95, 1)];

        assert!(select_delegate_peer(80, &peers).is_none());
    }
}
