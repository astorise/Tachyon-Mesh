mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

const OTEL_ENDPOINT_HEADER: &str = "x-tachyon-shadow-otel-endpoint";

#[derive(Debug, Deserialize)]
struct ShadowEvent {
    route: String,
    shadow_target: String,
    method: String,
    #[allow(dead_code)]
    uri: String,
    #[serde(default)]
    headers: Vec<(String, String)>,
    body_hex: String,
    primary_status: u16,
    #[serde(default)]
    primary_headers: Vec<(String, String)>,
    primary_body_sha256: String,
    #[serde(default)]
    trace_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct ShadowDiff {
    route: String,
    shadow_target: String,
    trace_id: Option<String>,
    status_match: bool,
    headers_match: bool,
    body_match: bool,
    primary_status: u16,
    shadow_status: u16,
    primary_body_sha256: String,
    shadow_body_sha256: String,
}

struct Component;

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        if !req.method.eq_ignore_ascii_case("POST") {
            return response(405, "Method Not Allowed");
        }

        let event = match serde_json::from_slice::<ShadowEvent>(&req.body) {
            Ok(event) => event,
            Err(error) => return response(400, format!("invalid shadow event: {error}")),
        };
        let body = match hex::decode(&event.body_hex) {
            Ok(body) => body,
            Err(error) => return response(400, format!("invalid shadow body: {error}")),
        };

        let shadow = match bindings::tachyon::mesh::outbound_http::send_request(
            &event.method,
            &shadow_url(&event),
            &event.headers,
            &body,
        ) {
            Ok(reply) => reply,
            Err(error) => return response(502, format!("shadow target failed: {error}")),
        };

        let diff = compare_shadow(&event, &shadow);
        if diff.status_match && diff.headers_match && diff.body_match {
            return response(202, "Accepted");
        }

        if let Some(endpoint) = header(&req.headers, OTEL_ENDPOINT_HEADER) {
            let metric = json!({
                "trace_id": diff.trace_id,
                "sampled": true,
                "path": diff.route,
                "status": 599,
                "total_duration_us": 0,
                "wasm_duration_us": 0,
                "host_overhead_us": 0,
                "shadow": diff,
            });
            let payload = match serde_json::to_vec(&metric) {
                Ok(mut payload) => {
                    payload.push(b'\n');
                    payload
                }
                Err(error) => return response(500, format!("failed to encode diff: {error}")),
            };
            let _ = bindings::tachyon::mesh::outbound_http::send_request(
                "POST",
                endpoint,
                &[("content-type".to_owned(), "application/x-ndjson".to_owned())],
                &payload,
            );
        }

        response(202, "Divergence recorded")
    }
}

fn shadow_url(event: &ShadowEvent) -> String {
    if event.shadow_target.starts_with("http://") || event.shadow_target.starts_with("https://") {
        event.shadow_target.clone()
    } else if event.shadow_target.starts_with('/') {
        format!("http://mesh{}", event.shadow_target)
    } else {
        format!("http://mesh/{}", event.shadow_target)
    }
}

fn compare_shadow(
    event: &ShadowEvent,
    shadow: &bindings::tachyon::mesh::outbound_http::Response,
) -> ShadowDiff {
    let shadow_hash = sha256_hex(&shadow.body);
    ShadowDiff {
        route: event.route.clone(),
        shadow_target: event.shadow_target.clone(),
        trace_id: event.trace_id.clone(),
        status_match: event.primary_status == shadow.status,
        headers_match: normalize_headers(&event.primary_headers)
            == normalize_headers(&shadow.headers),
        body_match: event.primary_body_sha256.eq_ignore_ascii_case(&shadow_hash),
        primary_status: event.primary_status,
        shadow_status: shadow.status,
        primary_body_sha256: event.primary_body_sha256.clone(),
        shadow_body_sha256: shadow_hash,
    }
}

fn normalize_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    let mut normalized = headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect::<Vec<_>>();
    normalized.sort();
    normalized
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
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
