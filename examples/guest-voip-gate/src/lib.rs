mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "faas-guest",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};

struct Component;

#[derive(Debug, Deserialize)]
struct StartCallRequest {
    client_a_addr: String,
    client_b_addr: String,
    #[serde(default = "default_timeout_seconds")]
    timeout_seconds: u32,
}

#[derive(Debug, Serialize)]
struct StartCallResponse {
    bridge_id: String,
    ip: String,
    port_a: u16,
    port_b: u16,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        if req.method != "POST" {
            return response(405, "unsupported method");
        }

        let request = match serde_json::from_slice::<StartCallRequest>(&req.body) {
            Ok(request) => request,
            Err(error) => return response(400, format!("invalid start call payload: {error}")),
        };

        let allocation = match bindings::tachyon::mesh::bridge_controller::create_bridge(
            &bindings::tachyon::mesh::bridge_controller::BridgeConfig {
                client_a_addr: request.client_a_addr,
                client_b_addr: request.client_b_addr,
                timeout_seconds: request.timeout_seconds,
            },
        ) {
            Ok(allocation) => allocation,
            Err(error) => return response(502, format!("bridge allocation failed: {error}")),
        };

        let body = match serde_json::to_vec(&StartCallResponse {
            bridge_id: allocation.bridge_id,
            ip: allocation.ip,
            port_a: allocation.port_a,
            port_b: allocation.port_b,
        }) {
            Ok(body) => body,
            Err(error) => {
                return response(
                    500,
                    format!("failed to encode start call response: {error}"),
                );
            }
        };

        bindings::exports::tachyon::mesh::handler::Response {
            status: 200,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body,
            trailers: vec![],
        }
    }
}

fn default_timeout_seconds() -> u32 {
    30
}

fn response(
    status: u16,
    body: impl Into<Vec<u8>>,
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: vec![],
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
    fn start_call_response_serializes_ip_and_ports() {
        let encoded = serde_json::to_value(StartCallResponse {
            bridge_id: "bridge-1".to_owned(),
            ip: "203.0.113.50".to_owned(),
            port_a: 10_000,
            port_b: 10_001,
        })
        .expect("start call response should serialize");

        assert_eq!(encoded["bridge_id"], "bridge-1");
        assert_eq!(encoded["ip"], "203.0.113.50");
        assert_eq!(encoded["port_a"], 10_000);
        assert_eq!(encoded["port_b"], 10_001);
    }
}
