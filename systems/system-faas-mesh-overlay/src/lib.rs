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

use serde::{Deserialize, Serialize};

const NODE_ID_ENV: &str = "NODE_ID";
const PEER_URLS_ENV: &str = "PEER_URLS";
const OVERLAY_PATH_ENV: &str = "OVERLAY_PATH";
const OVERLAY_SHARED_SECRET_ENV: &str = "OVERLAY_SHARED_SECRET";
const ROUTE_PATH_ENV: &str = "ROUTE_PATH";
const DEFAULT_NODE_ID: &str = "local-node";
const DEFAULT_OVERLAY_PATH: &str = "/system/mesh-overlay";
const DEFAULT_ROUTE_PATH: &str = "/api/generate";
const AUTH_HEADER: &str = "x-tachyon-overlay-auth";
const PEER_ID_HEADER: &str = "x-tachyon-peer-id";
const REQUEST_ROUTE_HEADER: &str = "x-tachyon-request-route";

static ROUTING_TABLE: OnceLock<Mutex<RoutingTable>> = OnceLock::new();

struct Component;

impl bindings::Guest for Component {
    fn on_tick() {
        if let Err(error) = discover_peers() {
            eprintln!("system-faas-mesh-overlay discovery failed: {error}");
        }
        if let Err(error) = publish_route_override() {
            eprintln!("system-faas-mesh-overlay route override failed: {error}");
        }
    }
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let path = request_path(&req.uri);
        match (req.method.as_str(), path.as_str()) {
            ("GET", "/heartbeat") | ("GET", "/") => json_response(200, &local_heartbeat()),
            ("POST", "/heartbeat") => receive_heartbeat(req),
            ("POST", "/get_best_peer") => get_best_peer(req),
            ("POST", "/forward_request") => forward_request(req),
            _ => response(404, b"unknown mesh-overlay endpoint".to_vec(), &[]),
        }
    }
}

fn receive_heartbeat(
    req: bindings::exports::tachyon::mesh::handler::Request,
) -> bindings::exports::tachyon::mesh::handler::Response {
    if let Err(error) = authorize_peer(&req.headers) {
        return response(401, error.into_bytes(), &[]);
    }

    match serde_json::from_slice::<HardwareHeartbeat>(&req.body) {
        Ok(heartbeat) => {
            upsert_peer(heartbeat);
            response(204, Vec::new(), &[])
        }
        Err(error) => response(400, format!("invalid heartbeat: {error}").into_bytes(), &[]),
    }
}

fn get_best_peer(
    req: bindings::exports::tachyon::mesh::handler::Request,
) -> bindings::exports::tachyon::mesh::handler::Response {
    let requirements = if req.body.is_empty() {
        PeerRequirements::default()
    } else {
        match serde_json::from_slice::<PeerRequirements>(&req.body) {
            Ok(requirements) => requirements,
            Err(error) => {
                return response(
                    400,
                    format!("invalid peer requirements: {error}").into_bytes(),
                    &[],
                )
            }
        }
    };
    let Some(peer) = table()
        .lock()
        .expect("mesh overlay routing table should not be poisoned")
        .best_peer(&requirements)
    else {
        return response(404, b"no capable mesh peer available".to_vec(), &[]);
    };
    json_response(200, &peer)
}

fn forward_request(
    req: bindings::exports::tachyon::mesh::handler::Request,
) -> bindings::exports::tachyon::mesh::handler::Response {
    if let Err(error) = authorize_peer(&req.headers) {
        return response(401, error.into_bytes(), &[]);
    }
    let peer_id = match header_value(&req.headers, PEER_ID_HEADER) {
        Some(peer_id) => peer_id.to_owned(),
        None => return response(400, b"missing peer id".to_vec(), &[]),
    };
    let route = header_value(&req.headers, REQUEST_ROUTE_HEADER).unwrap_or(DEFAULT_ROUTE_PATH);
    let Some(peer) = table()
        .lock()
        .expect("mesh overlay routing table should not be poisoned")
        .peer(&peer_id)
    else {
        return response(
            404,
            format!("peer `{peer_id}` is unknown").into_bytes(),
            &[],
        );
    };
    if !peer.secure {
        return response(
            403,
            format!("peer `{peer_id}` has no secure tunnel").into_bytes(),
            &[],
        );
    }

    let url = format!(
        "{}{}",
        peer.base_url.trim_end_matches('/'),
        normalize_route(route)
    );
    let mut headers = req.headers.clone();
    headers.retain(|(name, _)| {
        !name.eq_ignore_ascii_case(PEER_ID_HEADER)
            && !name.eq_ignore_ascii_case(REQUEST_ROUTE_HEADER)
            && !name.eq_ignore_ascii_case("host")
            && !name.eq_ignore_ascii_case("content-length")
    });
    match bindings::tachyon::mesh::outbound_http::send_request("POST", &url, &headers, &req.body) {
        Ok(remote) => response_with_header_fields(remote.status, remote.body, remote.headers),
        Err(error) => response(
            502,
            format!("mesh forward failed: {error}").into_bytes(),
            &[],
        ),
    }
}

fn discover_peers() -> Result<(), String> {
    for peer in parse_peer_entries(&std::env::var(PEER_URLS_ENV).unwrap_or_default()) {
        let url = format!(
            "{}{}/heartbeat",
            peer.trim_end_matches('/'),
            env_or_default(OVERLAY_PATH_ENV, DEFAULT_OVERLAY_PATH)
        );
        let response = bindings::tachyon::mesh::outbound_http::send_request(
            "GET",
            &url,
            &auth_headers(),
            &[],
        )?;
        if response.status >= 400 {
            continue;
        }
        if let Ok(mut heartbeat) = serde_json::from_slice::<HardwareHeartbeat>(&response.body) {
            heartbeat.base_url = peer;
            heartbeat.secure = heartbeat.base_url.starts_with("https://")
                || std::env::var(OVERLAY_SHARED_SECRET_ENV).is_ok();
            upsert_peer(heartbeat);
        }
    }
    Ok(())
}

fn publish_route_override() -> Result<(), String> {
    let candidates = table()
        .lock()
        .expect("mesh overlay routing table should not be poisoned")
        .candidates();
    if candidates.is_empty() {
        return Ok(());
    }
    let descriptor = RouteOverrideDescriptor { candidates };
    let payload = serde_json::to_string(&descriptor)
        .map_err(|error| format!("failed to encode route override descriptor: {error}"))?;
    bindings::tachyon::mesh::routing_control::update_target(
        &env_or_default(ROUTE_PATH_ENV, DEFAULT_ROUTE_PATH),
        &payload,
    )
}

fn local_heartbeat() -> HardwareHeartbeat {
    let snapshot = bindings::tachyon::mesh::telemetry_reader::get_metrics();
    HardwareHeartbeat {
        node_id: env_or_default(NODE_ID_ENV, DEFAULT_NODE_ID),
        status: "online".to_owned(),
        base_url: local_base_url(&snapshot.advertise_ip),
        hardware: Hardware {
            gpu: AcceleratorStatus {
                present: snapshot
                    .capabilities
                    .iter()
                    .any(|capability| capability.eq_ignore_ascii_case("gpu")),
                load_percent: snapshot
                    .gpu_rt_load
                    .saturating_add(snapshot.gpu_standard_load) as u8,
            },
            ram: RamStatus {
                free_mb: 0,
                pressure_percent: snapshot.ram_pressure,
            },
        },
        active_faas_count: snapshot.active_instances,
        supported_models: snapshot.hot_models,
        capability_mask: snapshot.capability_mask,
        capabilities: snapshot.capabilities,
        secure: std::env::var(OVERLAY_SHARED_SECRET_ENV).is_ok(),
        seen_at_unix_ms: now_ms(),
    }
}

fn local_base_url(advertise_ip: &str) -> String {
    std::env::var("PUBLIC_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| format!("https://{}", advertise_ip.trim()))
}

fn table() -> &'static Mutex<RoutingTable> {
    ROUTING_TABLE.get_or_init(|| Mutex::new(RoutingTable::default()))
}

fn upsert_peer(heartbeat: HardwareHeartbeat) {
    if heartbeat.node_id == env_or_default(NODE_ID_ENV, DEFAULT_NODE_ID) {
        return;
    }
    table()
        .lock()
        .expect("mesh overlay routing table should not be poisoned")
        .upsert(heartbeat);
}

fn authorize_peer(headers: &[(String, String)]) -> Result<(), String> {
    let Ok(secret) = std::env::var(OVERLAY_SHARED_SECRET_ENV) else {
        return Ok(());
    };
    match header_value(headers, AUTH_HEADER) {
        Some(value) if value == secret => Ok(()),
        _ => Err("unauthenticated mesh peer".to_owned()),
    }
}

fn auth_headers() -> Vec<(String, String)> {
    std::env::var(OVERLAY_SHARED_SECRET_ENV)
        .ok()
        .map(|secret| vec![(AUTH_HEADER.to_owned(), secret)])
        .unwrap_or_default()
}

fn request_path(uri: &str) -> String {
    let path = uri.split('?').next().unwrap_or(uri);
    if let Some(index) = path.rfind(DEFAULT_OVERLAY_PATH) {
        normalize_route(&path[index + DEFAULT_OVERLAY_PATH.len()..])
    } else {
        normalize_route(path)
    }
}

fn normalize_route(route: &str) -> String {
    let trimmed = route.trim();
    if trimmed.is_empty() || trimmed == "/" {
        "/".to_owned()
    } else if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    }
}

fn env_or_default(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_owned())
}

fn parse_peer_entries(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|entry| entry.starts_with("https://") || entry.starts_with("http://"))
        .map(|entry| entry.trim_end_matches('/').to_owned())
        .collect()
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

fn json_response<T: Serialize>(
    status: u16,
    value: &T,
) -> bindings::exports::tachyon::mesh::handler::Response {
    match serde_json::to_vec(value) {
        Ok(body) => response(status, body, &[("content-type", "application/json")]),
        Err(error) => response(
            500,
            format!("failed to encode mesh-overlay response: {error}").into_bytes(),
            &[],
        ),
    }
}

fn response(
    status: u16,
    body: Vec<u8>,
    headers: &[(&str, &str)],
) -> bindings::exports::tachyon::mesh::handler::Response {
    response_with_header_fields(
        status,
        body,
        headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
    )
}

fn response_with_header_fields(
    status: u16,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers,
        body,
        trailers: Vec::new(),
    }
}

#[derive(Clone, Debug, Default)]
struct RoutingTable {
    peers: HashMap<String, HardwareHeartbeat>,
}

impl RoutingTable {
    fn upsert(&mut self, heartbeat: HardwareHeartbeat) {
        self.peers.insert(heartbeat.node_id.clone(), heartbeat);
    }

    fn peer(&self, peer_id: &str) -> Option<HardwareHeartbeat> {
        self.peers.get(peer_id).cloned()
    }

    fn best_peer(&self, requirements: &PeerRequirements) -> Option<HardwareHeartbeat> {
        self.peers
            .values()
            .filter(|peer| peer.status == "online" && peer.secure)
            .filter(|peer| !requirements.gpu_required || peer.hardware.gpu.present)
            .filter(|peer| {
                requirements.supported_model.as_ref().is_none_or(|model| {
                    peer.supported_models
                        .iter()
                        .any(|candidate| candidate.eq_ignore_ascii_case(model))
                })
            })
            .min_by_key(|peer| {
                (
                    peer.hardware.gpu.load_percent,
                    peer.hardware.ram.pressure_percent,
                    peer.active_faas_count,
                )
            })
            .cloned()
    }

    fn candidates(&self) -> Vec<RouteOverrideCandidate> {
        self.peers
            .values()
            .filter(|peer| peer.status == "online" && peer.secure)
            .map(|peer| RouteOverrideCandidate {
                destination: format!(
                    "{}{}",
                    peer.base_url.trim_end_matches('/'),
                    DEFAULT_ROUTE_PATH
                ),
                hot_models: peer.supported_models.clone(),
                effective_pressure: peer
                    .hardware
                    .gpu
                    .load_percent
                    .max(peer.hardware.ram.pressure_percent),
                capability_mask: peer.capability_mask,
                capabilities: peer.capabilities.clone(),
            })
            .collect()
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
struct PeerRequirements {
    #[serde(default)]
    gpu_required: bool,
    #[serde(default)]
    supported_model: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct HardwareHeartbeat {
    node_id: String,
    status: String,
    base_url: String,
    hardware: Hardware,
    active_faas_count: u32,
    supported_models: Vec<String>,
    #[serde(default)]
    capability_mask: u64,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    secure: bool,
    seen_at_unix_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Hardware {
    gpu: AcceleratorStatus,
    ram: RamStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AcceleratorStatus {
    present: bool,
    load_percent: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RamStatus {
    free_mb: u64,
    pressure_percent: u8,
}

#[derive(Clone, Debug, Serialize)]
struct RouteOverrideDescriptor {
    candidates: Vec<RouteOverrideCandidate>,
}

#[derive(Clone, Debug, Serialize)]
struct RouteOverrideCandidate {
    destination: String,
    hot_models: Vec<String>,
    effective_pressure: u8,
    capability_mask: u64,
    capabilities: Vec<String>,
}
