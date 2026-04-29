mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "control-plane-faas",
    });

    export!(Component);
}

use serde::Serialize;

const TEE_BACKEND_AVAILABLE_ENV: &str = "TEE_BACKEND_AVAILABLE";

struct Component;

impl bindings::Guest for Component {
    fn on_tick() {}
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        match (req.method.as_str(), request_path(&req.uri).as_str()) {
            ("GET", "/attest") | ("GET", "/health") => attest(),
            _ => response(404, b"unknown tee-runtime endpoint".to_vec()),
        }
    }
}

fn attest() -> bindings::exports::tachyon::mesh::handler::Response {
    let available = std::env::var(TEE_BACKEND_AVAILABLE_ENV)
        .ok()
        .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes"));
    json_response(
        if available { 200 } else { 503 },
        &AttestationResponse {
            backend: if available {
                "hardware-tee"
            } else {
                "unavailable"
            },
            attested: available,
        },
    )
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
    value: &T,
) -> bindings::exports::tachyon::mesh::handler::Response {
    match serde_json::to_vec(value) {
        Ok(body) => bindings::exports::tachyon::mesh::handler::Response {
            status,
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            body,
            trailers: Vec::new(),
        },
        Err(error) => response(
            500,
            format!("failed to encode TEE attestation: {error}").into_bytes(),
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

#[derive(Serialize)]
struct AttestationResponse {
    backend: &'static str,
    attested: bool,
}
