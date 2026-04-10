mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};

const SESSION_DIR: &str = "/sessions";

struct Component;

#[derive(Debug, Deserialize)]
struct CreateBridgeRequest {
    client_a_addr: String,
    client_b_addr: String,
    #[serde(default = "default_timeout_seconds")]
    timeout_seconds: u32,
}

#[derive(Debug, Deserialize)]
struct DestroyBridgeRequest {
    bridge_id: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct BridgeResponse {
    bridge_id: String,
    port_a: u16,
    port_b: u16,
}

#[derive(Debug, Serialize)]
struct BridgeSessionRecord {
    bridge_id: String,
    client_a_addr: String,
    client_b_addr: String,
    timeout_seconds: u32,
    port_a: u16,
    port_b: u16,
    status: &'static str,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        match req.method.as_str() {
            "POST" => create_bridge(req.body),
            "DELETE" => destroy_bridge(req.body),
            _ => response(405, "unsupported method"),
        }
    }
}

fn create_bridge(body: Vec<u8>) -> bindings::exports::tachyon::mesh::handler::Response {
    let request = match serde_json::from_slice::<CreateBridgeRequest>(&body) {
        Ok(request) => request,
        Err(error) => return response(400, format!("invalid create bridge payload: {error}")),
    };

    let allocation = match bindings::tachyon::mesh::bridge_controller::create_bridge(
        &bindings::tachyon::mesh::bridge_controller::BridgeConfig {
            client_a_addr: request.client_a_addr.clone(),
            client_b_addr: request.client_b_addr.clone(),
            timeout_seconds: request.timeout_seconds,
        },
    ) {
        Ok(allocation) => allocation,
        Err(error) => return response(502, format!("bridge allocation failed: {error}")),
    };

    let session = BridgeSessionRecord {
        bridge_id: allocation.bridge_id.clone(),
        client_a_addr: request.client_a_addr,
        client_b_addr: request.client_b_addr,
        timeout_seconds: request.timeout_seconds,
        port_a: allocation.port_a,
        port_b: allocation.port_b,
        status: "active",
    };
    if let Err(error) = persist_session(&session) {
        return response(500, format!("failed to persist bridge session: {error}"));
    }

    let response_body = match serde_json::to_vec(&BridgeResponse {
        bridge_id: allocation.bridge_id,
        port_a: allocation.port_a,
        port_b: allocation.port_b,
    }) {
        Ok(body) => body,
        Err(error) => {
            return response(500, format!("failed to encode bridge response: {error}"));
        }
    };

    response_with_headers(
        200,
        response_body,
        &[("content-type", "application/json")],
    )
}

fn destroy_bridge(body: Vec<u8>) -> bindings::exports::tachyon::mesh::handler::Response {
    let request = match serde_json::from_slice::<DestroyBridgeRequest>(&body) {
        Ok(request) => request,
        Err(error) => return response(400, format!("invalid destroy bridge payload: {error}")),
    };

    if let Err(error) =
        bindings::tachyon::mesh::bridge_controller::destroy_bridge(&request.bridge_id)
    {
        return response(502, format!("bridge teardown failed: {error}"));
    }

    let session = BridgeSessionRecord {
        bridge_id: request.bridge_id,
        client_a_addr: String::new(),
        client_b_addr: String::new(),
        timeout_seconds: 0,
        port_a: 0,
        port_b: 0,
        status: "destroyed",
    };
    if let Err(error) = persist_session(&session) {
        return response(500, format!("failed to persist bridge teardown: {error}"));
    }

    response(204, Vec::<u8>::new())
}

fn persist_session(session: &BridgeSessionRecord) -> Result<(), String> {
    std::fs::create_dir_all(SESSION_DIR)
        .map_err(|error| format!("failed to create session dir: {error}"))?;
    let path = format!("{SESSION_DIR}/{}.json", session.bridge_id);
    let payload =
        serde_json::to_vec(session).map_err(|error| format!("failed to serialize session: {error}"))?;
    std::fs::write(path, payload).map_err(|error| format!("failed to write session file: {error}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_timeout_is_30_seconds() {
        assert_eq!(default_timeout_seconds(), 30);
    }

    #[test]
    fn bridge_response_serializes_json_shape() {
        let encoded = serde_json::to_value(BridgeResponse {
            bridge_id: "bridge-1".to_owned(),
            port_a: 10_000,
            port_b: 10_001,
        })
        .expect("bridge response should serialize");
        assert_eq!(encoded["bridge_id"], "bridge-1");
        assert_eq!(encoded["port_a"], 10_000);
        assert_eq!(encoded["port_b"], 10_001);
    }
}
