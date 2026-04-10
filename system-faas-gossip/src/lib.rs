use std::sync::atomic::{AtomicU64, Ordering};

mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../wit",
        world: "control-plane-faas",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};

const STEER_ROUTE_ENV: &str = "STEER_ROUTE";
const PEER_URLS_ENV: &str = "PEER_URLS";
const BUFFER_ROUTE_ENV: &str = "BUFFER_ROUTE";
const SOFT_LIMIT_ENV: &str = "SOFT_LIMIT";
const RECOVER_LIMIT_ENV: &str = "RECOVER_LIMIT";
const SATURATED_LIMIT_ENV: &str = "SATURATED_LIMIT";
const TARGET_ACCELERATOR_ENV: &str = "TARGET_ACCELERATOR";
const TARGET_QOS_ENV: &str = "TARGET_QOS";
const GOSSIP_PATH_ENV: &str = "GOSSIP_PATH";
const MESH_QOS_OVERRIDE_PREFIX: &str = "mesh-qos:";
const DEFAULT_BUFFER_ROUTE: &str = "/system/buffer";
const DEFAULT_GOSSIP_PATH: &str = "/system/gossip";
const DEFAULT_SOFT_LIMIT: u8 = 85;
const DEFAULT_RECOVER_LIMIT: u8 = 60;
const DEFAULT_SATURATED_LIMIT: u8 = 95;

static TICK_COUNTER: AtomicU64 = AtomicU64::new(0);

struct Component;

impl bindings::Guest for Component {
    fn on_tick() {
        if let Err(error) = evaluate_cluster_pressure() {
            eprintln!("system-faas-gossip tick failed: {error}");
        }
    }
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        _req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let snapshot = local_snapshot();
        match serde_json::to_vec(&snapshot) {
            Ok(body) => response(200, body, &[("content-type", "application/json")]),
            Err(error) => response(
                500,
                format!("failed to encode gossip snapshot: {error}").into_bytes(),
                &[],
            ),
        }
    }
}

fn evaluate_cluster_pressure() -> Result<(), String> {
    let steer_route = normalize_route(&required_env(STEER_ROUTE_ENV)?)?;
    let buffer_route = normalize_route(&env_or_default(BUFFER_ROUTE_ENV, DEFAULT_BUFFER_ROUTE))?;
    let soft_limit = parse_limit(SOFT_LIMIT_ENV, DEFAULT_SOFT_LIMIT)?;
    let recover_limit = parse_limit(RECOVER_LIMIT_ENV, DEFAULT_RECOVER_LIMIT)?;
    let saturated_limit = parse_limit(SATURATED_LIMIT_ENV, DEFAULT_SATURATED_LIMIT)?;
    let local = local_snapshot();
    let local_pressure = local.effective_pressure();
    let peers = fetch_peer_snapshots()?;

    if let Some(profile) = steering_profile_from_env()? {
        return evaluate_qos_routing(
            &steer_route,
            &buffer_route,
            recover_limit,
            saturated_limit,
            &local,
            &peers,
            profile,
        );
    }

    if local_pressure <= recover_limit {
        bindings::tachyon::mesh::routing_control::update_target(&steer_route, &steer_route)?;
        return Ok(());
    }

    if local_pressure < soft_limit {
        return Ok(());
    }
    let healthy_peers = peers
        .into_iter()
        .filter(|peer| peer.snapshot.effective_pressure() < saturated_limit)
        .collect::<Vec<_>>();

    if healthy_peers.is_empty() {
        bindings::tachyon::mesh::routing_control::update_target(&steer_route, &buffer_route)?;
        return Ok(());
    }

    let ordered_candidates = ordered_candidates(&healthy_peers);
    let Some(choice) = ordered_candidates.first() else {
        return Ok(());
    };

    if choice.snapshot.effective_pressure() >= local_pressure && local_pressure < saturated_limit {
        return Ok(());
    }

    let payload = serde_json::to_string(&RouteOverrideDescriptor {
        candidates: ordered_candidates
            .iter()
            .map(|candidate| RouteOverrideCandidate {
                destination: format!(
                    "{}{}",
                    candidate.base_url,
                    route_path_for_override_key(&steer_route)
                ),
                hot_models: candidate.snapshot.hot_models.clone(),
                effective_pressure: candidate.snapshot.effective_pressure(),
                capability_mask: candidate.snapshot.capability_mask,
                capabilities: candidate.snapshot.capabilities.clone(),
            })
            .collect(),
    })
    .map_err(|error| format!("failed to encode route override descriptor: {error}"))?;

    bindings::tachyon::mesh::routing_control::update_target(&steer_route, &payload)
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
        total_requests: snapshot.total_requests,
        completed_requests: snapshot.completed_requests,
        error_requests: snapshot.error_requests,
        active_requests: snapshot.active_requests,
        cpu_pressure: snapshot.cpu_pressure,
        ram_pressure: snapshot.ram_pressure,
        active_instances: snapshot.active_instances,
        allocated_memory_pages: snapshot.allocated_memory_pages,
        capability_mask: snapshot.capability_mask,
        capabilities: snapshot.capabilities,
        cpu_rt_load: snapshot.cpu_rt_load,
        cpu_standard_load: snapshot.cpu_standard_load,
        cpu_batch_load: snapshot.cpu_batch_load,
        gpu_rt_load: snapshot.gpu_rt_load,
        gpu_standard_load: snapshot.gpu_standard_load,
        gpu_batch_load: snapshot.gpu_batch_load,
        npu_rt_load: snapshot.npu_rt_load,
        npu_standard_load: snapshot.npu_standard_load,
        npu_batch_load: snapshot.npu_batch_load,
        tpu_rt_load: snapshot.tpu_rt_load,
        tpu_standard_load: snapshot.tpu_standard_load,
        tpu_batch_load: snapshot.tpu_batch_load,
        hot_models: snapshot.hot_models,
        dropped_events: snapshot.dropped_events,
        last_status: snapshot.last_status,
        total_duration_us: snapshot.total_duration_us,
        total_wasm_duration_us: snapshot.total_wasm_duration_us,
        total_host_overhead_us: snapshot.total_host_overhead_us,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetAccelerator {
    Cpu,
    Gpu,
    Npu,
    Tpu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TargetQos {
    RealTime,
    Standard,
    Batch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SteeringProfile {
    accelerator: TargetAccelerator,
    qos: TargetQos,
}

fn steering_profile_from_env() -> Result<Option<SteeringProfile>, String> {
    let Some(accelerator) = std::env::var(TARGET_ACCELERATOR_ENV)
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let qos = std::env::var(TARGET_QOS_ENV)
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "standard".to_owned());
    Ok(Some(SteeringProfile {
        accelerator: parse_target_accelerator(&accelerator)?,
        qos: parse_target_qos(&qos)?,
    }))
}

fn parse_target_accelerator(value: &str) -> Result<TargetAccelerator, String> {
    match value {
        "cpu" => Ok(TargetAccelerator::Cpu),
        "gpu" => Ok(TargetAccelerator::Gpu),
        "npu" => Ok(TargetAccelerator::Npu),
        "tpu" => Ok(TargetAccelerator::Tpu),
        _ => Err(format!(
            "unsupported `{TARGET_ACCELERATOR_ENV}` value `{value}`"
        )),
    }
}

fn parse_target_qos(value: &str) -> Result<TargetQos, String> {
    match value {
        "realtime" | "real-time" | "real_time" => Ok(TargetQos::RealTime),
        "standard" => Ok(TargetQos::Standard),
        "batch" => Ok(TargetQos::Batch),
        _ => Err(format!("unsupported `{TARGET_QOS_ENV}` value `{value}`")),
    }
}

fn queue_load(snapshot: &TelemetrySnapshot, accelerator: TargetAccelerator, qos: TargetQos) -> u32 {
    match (accelerator, qos) {
        (TargetAccelerator::Cpu, TargetQos::RealTime) => snapshot.cpu_rt_load,
        (TargetAccelerator::Cpu, TargetQos::Standard) => snapshot.cpu_standard_load,
        (TargetAccelerator::Cpu, TargetQos::Batch) => snapshot.cpu_batch_load,
        (TargetAccelerator::Gpu, TargetQos::RealTime) => snapshot.gpu_rt_load,
        (TargetAccelerator::Gpu, TargetQos::Standard) => snapshot.gpu_standard_load,
        (TargetAccelerator::Gpu, TargetQos::Batch) => snapshot.gpu_batch_load,
        (TargetAccelerator::Npu, TargetQos::RealTime) => snapshot.npu_rt_load,
        (TargetAccelerator::Npu, TargetQos::Standard) => snapshot.npu_standard_load,
        (TargetAccelerator::Npu, TargetQos::Batch) => snapshot.npu_batch_load,
        (TargetAccelerator::Tpu, TargetQos::RealTime) => snapshot.tpu_rt_load,
        (TargetAccelerator::Tpu, TargetQos::Standard) => snapshot.tpu_standard_load,
        (TargetAccelerator::Tpu, TargetQos::Batch) => snapshot.tpu_batch_load,
    }
}

fn evaluate_qos_routing(
    steer_route: &str,
    buffer_route: &str,
    recover_limit: u8,
    saturated_limit: u8,
    local: &TelemetrySnapshot,
    peers: &[PeerCandidate],
    profile: SteeringProfile,
) -> Result<(), String> {
    let local_load = queue_load(local, profile.accelerator, profile.qos);
    let healthy_peers = peers
        .iter()
        .filter(|peer| peer.snapshot.effective_pressure() < saturated_limit)
        .collect::<Vec<_>>();

    match profile.qos {
        TargetQos::RealTime => {
            if local_load == 0 && local.effective_pressure() <= recover_limit {
                bindings::tachyon::mesh::routing_control::update_target(steer_route, steer_route)?;
                return Ok(());
            }

            let ordered = qos_ordered_candidates(
                &healthy_peers,
                profile.accelerator,
                profile.qos,
                local_load,
            );

            if ordered.is_empty() {
                if local.effective_pressure() >= saturated_limit {
                    bindings::tachyon::mesh::routing_control::update_target(
                        steer_route,
                        buffer_route,
                    )?;
                }
                return Ok(());
            }

            let payload = serde_json::to_string(&RouteOverrideDescriptor {
                candidates: ordered
                    .iter()
                    .map(|candidate| RouteOverrideCandidate {
                        destination: format!(
                            "{}{}",
                            candidate.base_url,
                            route_path_for_override_key(steer_route)
                        ),
                        hot_models: candidate.snapshot.hot_models.clone(),
                        effective_pressure: candidate.snapshot.effective_pressure(),
                        capability_mask: candidate.snapshot.capability_mask,
                        capabilities: candidate.snapshot.capabilities.clone(),
                    })
                    .collect(),
            })
            .map_err(|error| format!("failed to encode route override descriptor: {error}"))?;
            bindings::tachyon::mesh::routing_control::update_target(steer_route, &payload)?;
        }
        TargetQos::Batch => {
            if local_load >= 1000 || local.effective_pressure() >= saturated_limit {
                bindings::tachyon::mesh::routing_control::update_target(steer_route, buffer_route)?;
            } else {
                bindings::tachyon::mesh::routing_control::update_target(steer_route, steer_route)?;
            }
        }
        TargetQos::Standard => {
            if local_load == 0 && local.effective_pressure() <= recover_limit {
                bindings::tachyon::mesh::routing_control::update_target(steer_route, steer_route)?;
                return Ok(());
            }
            let ordered = qos_ordered_candidates(
                &healthy_peers,
                profile.accelerator,
                profile.qos,
                local_load,
            );
            if let Some(candidate) = ordered.first() {
                let payload = serde_json::to_string(&RouteOverrideDescriptor {
                    candidates: vec![RouteOverrideCandidate {
                        destination: format!(
                            "{}{}",
                            candidate.base_url,
                            route_path_for_override_key(steer_route)
                        ),
                        hot_models: candidate.snapshot.hot_models.clone(),
                        effective_pressure: candidate.snapshot.effective_pressure(),
                        capability_mask: candidate.snapshot.capability_mask,
                        capabilities: candidate.snapshot.capabilities.clone(),
                    }],
                })
                .map_err(|error| format!("failed to encode route override descriptor: {error}"))?;
                bindings::tachyon::mesh::routing_control::update_target(steer_route, &payload)?;
            }
        }
    }

    Ok(())
}

fn qos_ordered_candidates<'a>(
    peers: &'a [&'a PeerCandidate],
    accelerator: TargetAccelerator,
    qos: TargetQos,
    local_load: u32,
) -> Vec<&'a PeerCandidate> {
    let mut ordered = peers
        .iter()
        .copied()
        .filter(|peer| queue_load(&peer.snapshot, accelerator, qos) < local_load)
        .collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        queue_load(&left.snapshot, accelerator, qos)
            .cmp(&queue_load(&right.snapshot, accelerator, qos))
            .then_with(|| {
                left.snapshot
                    .effective_pressure()
                    .cmp(&right.snapshot.effective_pressure())
            })
            .then_with(|| {
                left.snapshot
                    .active_requests
                    .cmp(&right.snapshot.active_requests)
            })
    });
    ordered
}

fn power_of_two_choices(peers: &[PeerCandidate]) -> Option<&PeerCandidate> {
    if peers.is_empty() {
        return None;
    }
    if peers.len() == 1 {
        return peers.first();
    }

    let seed = TICK_COUNTER.fetch_add(1, Ordering::Relaxed) as usize;
    let first = &peers[seed % peers.len()];
    let second = &peers[(seed + 1) % peers.len()];
    Some(prefer_healthier_peer(first, second))
}

fn ordered_candidates(peers: &[PeerCandidate]) -> Vec<&PeerCandidate> {
    let mut ordered = Vec::new();
    if let Some(choice) = power_of_two_choices(peers) {
        ordered.push(choice);
    }

    let mut remaining = peers.iter().collect::<Vec<_>>();
    remaining.retain(|peer| {
        ordered
            .iter()
            .all(|selected| selected.base_url != peer.base_url)
    });
    remaining.sort_by(|left, right| {
        left.snapshot
            .effective_pressure()
            .cmp(&right.snapshot.effective_pressure())
            .then_with(|| {
                left.snapshot
                    .active_requests
                    .cmp(&right.snapshot.active_requests)
            })
    });
    ordered.extend(remaining);
    ordered
}

fn prefer_healthier_peer<'a>(
    left: &'a PeerCandidate,
    right: &'a PeerCandidate,
) -> &'a PeerCandidate {
    let left_score = left.snapshot.effective_pressure();
    let right_score = right.snapshot.effective_pressure();
    match left_score.cmp(&right_score) {
        std::cmp::Ordering::Less => left,
        std::cmp::Ordering::Greater => right,
        std::cmp::Ordering::Equal => {
            if left.snapshot.active_requests <= right.snapshot.active_requests {
                left
            } else {
                right
            }
        }
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
    if let Some(route) = route.strip_prefix(MESH_QOS_OVERRIDE_PREFIX) {
        return Ok(format!(
            "{MESH_QOS_OVERRIDE_PREFIX}{}",
            normalize_route(route)?
        ));
    }

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

fn route_path_for_override_key(route: &str) -> String {
    route
        .strip_prefix(MESH_QOS_OVERRIDE_PREFIX)
        .map(normalize_route)
        .transpose()
        .ok()
        .flatten()
        .unwrap_or_else(|| normalize_route(route).unwrap_or_else(|_| "/".to_owned()))
}

fn required_env(name: &str) -> Result<String, String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("missing required environment variable `{name}`"))
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

fn response(
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

#[derive(Clone, Debug, Deserialize, Serialize)]
struct TelemetrySnapshot {
    total_requests: u64,
    completed_requests: u64,
    error_requests: u64,
    active_requests: u32,
    cpu_pressure: u8,
    ram_pressure: u8,
    active_instances: u32,
    allocated_memory_pages: u32,
    #[serde(default)]
    capability_mask: u64,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    cpu_rt_load: u32,
    #[serde(default)]
    cpu_standard_load: u32,
    #[serde(default)]
    cpu_batch_load: u32,
    #[serde(default)]
    gpu_rt_load: u32,
    #[serde(default)]
    gpu_standard_load: u32,
    #[serde(default)]
    gpu_batch_load: u32,
    #[serde(default)]
    npu_rt_load: u32,
    #[serde(default)]
    npu_standard_load: u32,
    #[serde(default)]
    npu_batch_load: u32,
    #[serde(default)]
    tpu_rt_load: u32,
    #[serde(default)]
    tpu_standard_load: u32,
    #[serde(default)]
    tpu_batch_load: u32,
    #[serde(default)]
    hot_models: Vec<String>,
    dropped_events: u64,
    last_status: u16,
    total_duration_us: u64,
    total_wasm_duration_us: u64,
    total_host_overhead_us: u64,
}

impl TelemetrySnapshot {
    fn effective_pressure(&self) -> u8 {
        self.cpu_pressure.max(self.ram_pressure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(cpu: u8, ram: u8, active_requests: u32) -> TelemetrySnapshot {
        TelemetrySnapshot {
            total_requests: 0,
            completed_requests: 0,
            error_requests: 0,
            active_requests,
            cpu_pressure: cpu,
            ram_pressure: ram,
            active_instances: 0,
            allocated_memory_pages: 0,
            capability_mask: 0,
            capabilities: Vec::new(),
            cpu_rt_load: 0,
            cpu_standard_load: 0,
            cpu_batch_load: 0,
            gpu_rt_load: 0,
            gpu_standard_load: 0,
            gpu_batch_load: 0,
            npu_rt_load: 0,
            npu_standard_load: 0,
            npu_batch_load: 0,
            tpu_rt_load: 0,
            tpu_standard_load: 0,
            tpu_batch_load: 0,
            hot_models: Vec::new(),
            dropped_events: 0,
            last_status: 0,
            total_duration_us: 0,
            total_wasm_duration_us: 0,
            total_host_overhead_us: 0,
        }
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
    fn p2c_prefers_lower_pressure_then_lower_activity() {
        TICK_COUNTER.store(0, Ordering::Relaxed);
        let peers = vec![
            PeerCandidate {
                base_url: "http://node-a".to_owned(),
                snapshot: snapshot(90, 40, 10),
            },
            PeerCandidate {
                base_url: "http://node-b".to_owned(),
                snapshot: snapshot(30, 20, 5),
            },
        ];

        let choice = power_of_two_choices(&peers).expect("a peer should be selected");
        assert_eq!(choice.base_url, "http://node-b");
    }

    #[test]
    fn ordered_candidates_puts_p2c_winner_first_and_preserves_other_peers() {
        TICK_COUNTER.store(0, Ordering::Relaxed);
        let peers = vec![
            PeerCandidate {
                base_url: "http://node-a".to_owned(),
                snapshot: snapshot(70, 70, 5),
            },
            PeerCandidate {
                base_url: "http://node-b".to_owned(),
                snapshot: snapshot(20, 20, 1),
            },
            PeerCandidate {
                base_url: "http://node-c".to_owned(),
                snapshot: snapshot(30, 30, 2),
            },
        ];

        let ordered = ordered_candidates(&peers);
        assert_eq!(ordered.len(), 3);
        assert_eq!(ordered[0].base_url, "http://node-b");
        assert!(ordered.iter().any(|peer| peer.base_url == "http://node-c"));
    }

    #[test]
    fn route_override_candidates_carry_capability_metadata() {
        let snapshot = TelemetrySnapshot {
            total_requests: 0,
            completed_requests: 0,
            error_requests: 0,
            active_requests: 1,
            cpu_pressure: 15,
            ram_pressure: 10,
            active_instances: 1,
            allocated_memory_pages: 1,
            capability_mask: 5,
            capabilities: vec!["core:wasi".to_owned(), "accel:cuda".to_owned()],
            cpu_rt_load: 0,
            cpu_standard_load: 0,
            cpu_batch_load: 0,
            gpu_rt_load: 0,
            gpu_standard_load: 0,
            gpu_batch_load: 0,
            npu_rt_load: 0,
            npu_standard_load: 0,
            npu_batch_load: 0,
            tpu_rt_load: 0,
            tpu_standard_load: 0,
            tpu_batch_load: 0,
            hot_models: vec!["llama3".to_owned()],
            dropped_events: 0,
            last_status: 200,
            total_duration_us: 0,
            total_wasm_duration_us: 0,
            total_host_overhead_us: 0,
        };
        let descriptor = RouteOverrideDescriptor {
            candidates: vec![RouteOverrideCandidate {
                destination: "http://node-a/api/guest-ai".to_owned(),
                hot_models: snapshot.hot_models.clone(),
                effective_pressure: snapshot.effective_pressure(),
                capability_mask: snapshot.capability_mask,
                capabilities: snapshot.capabilities.clone(),
            }],
        };

        let encoded = serde_json::to_value(descriptor).expect("descriptor should serialize");
        assert_eq!(encoded["candidates"][0]["capability_mask"], 5);
        assert_eq!(encoded["candidates"][0]["capabilities"][0], "core:wasi");
        assert_eq!(encoded["candidates"][0]["capabilities"][1], "accel:cuda");
    }

    #[test]
    fn qos_ordered_candidates_prefers_lower_gpu_realtime_backlog() {
        let peers = [
            PeerCandidate {
                base_url: "http://node-a".to_owned(),
                snapshot: TelemetrySnapshot {
                    gpu_rt_load: 4,
                    ..snapshot(40, 20, 4)
                },
            },
            PeerCandidate {
                base_url: "http://node-b".to_owned(),
                snapshot: TelemetrySnapshot {
                    gpu_rt_load: 0,
                    ..snapshot(30, 20, 2)
                },
            },
            PeerCandidate {
                base_url: "http://node-c".to_owned(),
                snapshot: TelemetrySnapshot {
                    gpu_rt_load: 1,
                    ..snapshot(10, 10, 1)
                },
            },
        ];

        let peer_refs = peers.iter().collect::<Vec<_>>();
        let ordered =
            qos_ordered_candidates(&peer_refs, TargetAccelerator::Gpu, TargetQos::RealTime, 5);

        assert_eq!(ordered[0].base_url, "http://node-b");
        assert_eq!(ordered[1].base_url, "http://node-c");
    }

    #[test]
    fn queue_load_reads_batch_tier_without_mixing_qos() {
        let snapshot = TelemetrySnapshot {
            gpu_rt_load: 1,
            gpu_standard_load: 2,
            gpu_batch_load: 7,
            ..snapshot(20, 20, 1)
        };

        assert_eq!(
            queue_load(&snapshot, TargetAccelerator::Gpu, TargetQos::Batch),
            7
        );
        assert_eq!(
            queue_load(&snapshot, TargetAccelerator::Gpu, TargetQos::RealTime),
            1
        );
    }
}
