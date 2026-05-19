mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "control-plane-faas",
    });

    export!(Component);
}

pub mod enrollment;
pub mod types;

include!(concat!(env!("OUT_DIR"), "/static_catalog.rs"));

use enrollment::{deterministic_pin, node_id_from_public_key, EnrollmentSession};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use types::{EnrolledNode, NodeCapabilities};

const REGISTRY_TABLE: &str = "node-registry";
const SESSION_KEY_PREFIX: &str = "enrollment-session:";
const ROUTE_ENROLLMENT_START: &str = "/admin/enrollment/start";
const ROUTE_ENROLLMENT_APPROVE: &str = "/admin/enrollment/approve";
const ROUTE_ENROLLMENT_POLL_PREFIX: &str = "/admin/enrollment/poll/";
const ROUTE_NODES: &str = "/admin/nodes";
const ROUTE_SYSTEMS_REGISTERED: &str = "/admin/systems/registered";
const ROUTE_SYSTEMS_DEPLOYED: &str = "/admin/systems/deployed";

struct Component;

thread_local! {
    static REGISTRY_CACHE: RefCell<Option<Vec<EnrolledNode>>> = const { RefCell::new(None) };
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnrollmentStartRequest {
    node_public_key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct EnrollmentStartResponse {
    session_id: String,
    pin: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnrollmentApproveRequest {
    #[serde(default)]
    session_id: String,
    pin: String,
    signed_certificate_hex: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoredEnrollmentSession {
    session_id: String,
    node_public_key: String,
    pin: String,
    created_at: u64,
    signed_certificate_hex: Option<String>,
    consumed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegisteredSystem {
    slug: String,
    crate_name: String,
    version: String,
    description: String,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct DeployedSystem {
    slug: String,
    catalog_version: String,
    status: String,
    host_node_count: usize,
    has_drift: bool,
    nodes: Vec<ActiveSystemNode>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct ActiveSystemNode {
    node_id: String,
    version: String,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let result = route_request(&req.method, route_path(&req.uri), &req.body);
        match result {
            Ok((status, body)) => response(status, body),
            Err(error) => response(400, error.into_bytes()),
        }
    }
}

impl bindings::Guest for Component {
    fn on_tick() {
        let _ = Registry::new().mark_stale_after(15 * 60);
    }
}

fn route_request(method: &str, path: &str, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let registry = Registry::new();
    if method.eq_ignore_ascii_case("POST") && path == ROUTE_ENROLLMENT_START {
        let input: EnrollmentStartRequest = serde_json::from_slice(body)
            .map_err(|error| format!("invalid enrollment request: {error}"))?;
        let session_id = format!("enroll-{}", deterministic_pin(&input.node_public_key));
        let session = EnrollmentSession {
            session_id: session_id.clone(),
            node_public_key: input.node_public_key.clone(),
            pin: deterministic_pin(&session_id),
            created_at: now_seconds(),
        };
        registry.write_enrollment_session(&StoredEnrollmentSession {
            session_id: session.session_id.clone(),
            node_public_key: session.node_public_key,
            pin: session.pin.clone(),
            created_at: session.created_at,
            signed_certificate_hex: None,
            consumed: false,
        })?;
        return json(
            201,
            &EnrollmentStartResponse {
                session_id,
                pin: session.pin,
            },
        );
    }
    if method.eq_ignore_ascii_case("POST") && path.starts_with(ROUTE_ENROLLMENT_APPROVE) {
        let mut input: EnrollmentApproveRequest = serde_json::from_slice(body)
            .map_err(|error| format!("invalid enrollment approval request: {error}"))?;
        if input.session_id.trim().is_empty() {
            input.session_id = path
                .trim_start_matches(ROUTE_ENROLLMENT_APPROVE)
                .trim_matches('/')
                .to_owned();
        }
        if input.session_id.trim().is_empty() {
            return Err("sessionId is required".to_owned());
        }
        let mut session = registry
            .read_enrollment_session(&input.session_id)?
            .ok_or_else(|| "unknown enrollment session".to_owned())?;
        if session.consumed || session.signed_certificate_hex.is_some() {
            return Err("enrollment session is already finalized".to_owned());
        }
        if session.pin != input.pin {
            return Err("invalid enrollment PIN".to_owned());
        }
        hex::decode(&input.signed_certificate_hex)
            .map_err(|error| format!("signedCertificateHex must be valid hex: {error}"))?;
        session.signed_certificate_hex = Some(input.signed_certificate_hex);
        registry.write_enrollment_session(&session)?;
        registry.record_approval(&EnrollmentSession {
            session_id: session.session_id,
            node_public_key: session.node_public_key,
            pin: session.pin,
            created_at: session.created_at,
        })?;
        return Ok((202, b"enrollment approved".to_vec()));
    }
    if method.eq_ignore_ascii_case("GET") && path.starts_with(ROUTE_ENROLLMENT_POLL_PREFIX) {
        let session_id = path
            .trim_start_matches(ROUTE_ENROLLMENT_POLL_PREFIX)
            .trim_matches('/');
        let mut session = registry
            .read_enrollment_session(session_id)?
            .ok_or_else(|| "unknown enrollment session".to_owned())?;
        let Some(cert) = session.signed_certificate_hex.clone() else {
            return Ok((204, Vec::new()));
        };
        if session.consumed {
            return Ok((204, Vec::new()));
        }
        session.consumed = true;
        registry.write_enrollment_session(&session)?;
        return Ok((200, cert.into_bytes()));
    }
    if method.eq_ignore_ascii_case("GET") && path == ROUTE_NODES {
        return json(200, &registry.list()?);
    }
    if method.eq_ignore_ascii_case("POST")
        && path.starts_with("/admin/nodes/")
        && path.ends_with("/capabilities")
    {
        let node_id = path
            .trim_start_matches("/admin/nodes/")
            .trim_end_matches("/capabilities")
            .trim_matches('/');
        let capabilities: NodeCapabilities = serde_json::from_slice(body)
            .map_err(|error| format!("invalid capabilities payload: {error}"))?;
        registry.update_capabilities(node_id, capabilities)?;
        return Ok((204, Vec::new()));
    }
    if method.eq_ignore_ascii_case("GET") && path == ROUTE_SYSTEMS_REGISTERED {
        return json(200, &list_registered_systems());
    }
    if method.eq_ignore_ascii_case("GET") && path == ROUTE_SYSTEMS_DEPLOYED {
        return json(200, &list_deployed_systems(&registry.list()?));
    }
    Err(format!("unsupported node registry route `{method} {path}`"))
}

pub struct Registry;

impl Registry {
    pub fn new() -> Self {
        Self
    }

    pub fn record_approval(&self, session: &EnrollmentSession) -> Result<(), String> {
        let node_id = node_id_from_public_key(&session.node_public_key);
        let node = EnrolledNode::awaiting_capabilities(
            node_id.clone(),
            session.node_public_key.clone(),
            now_seconds(),
        );
        self.write_node(&node_id, &node)
    }

    pub fn update_capabilities(
        &self,
        node_id: &str,
        capabilities: NodeCapabilities,
    ) -> Result<(), String> {
        let mut node = self
            .get(node_id)?
            .ok_or_else(|| format!("unknown node `{node_id}`"))?;
        node.capabilities = capabilities;
        node.status = "online".to_owned();
        node.last_seen = now_seconds();
        self.write_node(node_id, &node)
    }

    pub fn list(&self) -> Result<Vec<EnrolledNode>, String> {
        if let Some(cached) = REGISTRY_CACHE.with(|cache| cache.borrow().clone()) {
            return Ok(cached);
        }
        let table = bindings::tachyon::mesh::kv_partition::Table::new(REGISTRY_TABLE);
        let rows = table
            .get_range("", "\u{10ffff}", 10_000, 0)
            .map_err(|error| format!("node registry scan failed: {error}"))?;
        let nodes = rows
            .into_iter()
            .filter_map(|(_, value)| serde_json::from_slice::<EnrolledNode>(&value).ok())
            .collect::<Vec<_>>();
        REGISTRY_CACHE.with(|cache| *cache.borrow_mut() = Some(nodes.clone()));
        Ok(nodes)
    }

    pub fn get(&self, node_id: &str) -> Result<Option<EnrolledNode>, String> {
        let table = bindings::tachyon::mesh::kv_partition::Table::new(REGISTRY_TABLE);
        match table.get(node_id) {
            Ok(value) => serde_json::from_slice(&value)
                .map(Some)
                .map_err(|error| format!("node registry row `{node_id}` is invalid: {error}")),
            Err(_) => Ok(None),
        }
    }

    pub fn mark_stale_after(&self, threshold_seconds: u64) -> Result<(), String> {
        let now = now_seconds();
        for mut node in self.list()? {
            if node.status == "online" && now.saturating_sub(node.last_seen) > threshold_seconds {
                node.status = "stale".to_owned();
                self.write_node(&node.node_id.clone(), &node)?;
            }
        }
        Ok(())
    }

    fn write_node(&self, node_id: &str, node: &EnrolledNode) -> Result<(), String> {
        let table = bindings::tachyon::mesh::kv_partition::Table::new(REGISTRY_TABLE);
        let value = serde_json::to_vec(node)
            .map_err(|error| format!("failed to encode node registry row: {error}"))?;
        table
            .set(node_id, &value)
            .map_err(|error| format!("node registry write failed: {error}"))?;
        REGISTRY_CACHE.with(|cache| *cache.borrow_mut() = None);
        Ok(())
    }

    fn read_enrollment_session(
        &self,
        session_id: &str,
    ) -> Result<Option<StoredEnrollmentSession>, String> {
        let table = bindings::tachyon::mesh::kv_partition::Table::new(REGISTRY_TABLE);
        match table.get(&session_key(session_id)) {
            Ok(value) => serde_json::from_slice(&value)
                .map(Some)
                .map_err(|error| format!("enrollment session `{session_id}` is invalid: {error}")),
            Err(_) => Ok(None),
        }
    }

    fn write_enrollment_session(&self, session: &StoredEnrollmentSession) -> Result<(), String> {
        let table = bindings::tachyon::mesh::kv_partition::Table::new(REGISTRY_TABLE);
        let value = serde_json::to_vec(session)
            .map_err(|error| format!("failed to encode enrollment session: {error}"))?;
        table
            .set(&session_key(&session.session_id), &value)
            .map_err(|error| format!("enrollment session write failed: {error}"))
    }
}

fn list_registered_systems() -> Vec<RegisteredSystem> {
    REGISTERED_SYSTEMS
        .iter()
        .map(|entry| RegisteredSystem {
            slug: entry.slug.to_owned(),
            crate_name: entry.crate_name.to_owned(),
            version: entry.version.to_owned(),
            description: entry.description.to_owned(),
        })
        .collect()
}

fn list_deployed_systems(nodes: &[EnrolledNode]) -> Vec<DeployedSystem> {
    REGISTERED_SYSTEMS
        .iter()
        .map(|entry| {
            let active = nodes
                .iter()
                .flat_map(|node| {
                    node.capabilities
                        .active_systems
                        .iter()
                        .filter(move |system| system.slug == entry.slug)
                        .map(move |system| ActiveSystemNode {
                            node_id: node.node_id.clone(),
                            version: system.version.clone(),
                        })
                })
                .collect::<Vec<_>>();
            let has_drift = active.iter().any(|node| node.version != entry.version);
            DeployedSystem {
                slug: entry.slug.to_owned(),
                catalog_version: entry.version.to_owned(),
                status: if active.is_empty() {
                    "not-deployed".to_owned()
                } else if has_drift {
                    "version-drift".to_owned()
                } else {
                    "deployed".to_owned()
                },
                host_node_count: active.len(),
                has_drift,
                nodes: active,
            }
        })
        .collect()
}

fn route_path(uri: &str) -> &str {
    uri.split_once('?').map(|(path, _)| path).unwrap_or(uri)
}

fn session_key(session_id: &str) -> String {
    format!("{SESSION_KEY_PREFIX}{session_id}")
}

fn json<T: Serialize>(status: u16, value: &T) -> Result<(u16, Vec<u8>), String> {
    serde_json::to_vec(value)
        .map(|body| (status, body))
        .map_err(|error| format!("failed to encode node registry response: {error}"))
}

fn response(status: u16, body: Vec<u8>) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body,
        trailers: vec![],
    }
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ActiveSystem;

    #[test]
    fn deployed_catalog_detects_version_drift() {
        let mut node = EnrolledNode::awaiting_capabilities("node-a".to_owned(), "pk".to_owned(), 1);
        node.capabilities.active_systems = vec![ActiveSystem {
            slug: "gateway".to_owned(),
            version: "0.9.0".to_owned(),
        }];

        let deployed = list_deployed_systems(&[node]);
        let gateway = deployed
            .iter()
            .find(|system| system.slug == "gateway")
            .expect("gateway should be in catalog");

        assert_eq!(gateway.status, "version-drift");
        assert!(gateway.has_drift);
        assert_eq!(gateway.host_node_count, 1);
    }
}
