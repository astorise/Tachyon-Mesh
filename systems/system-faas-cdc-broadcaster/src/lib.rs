mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};

struct Component;

#[derive(Debug, Deserialize, Serialize)]
struct MutationEvent {
    namespace: String,
    key: String,
    op: String,
    #[serde(default)]
    vector_clock: u64,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        match filter_event(&req.body, header_value(&req.headers, "authorization")) {
            Ok(body) => response(200, body),
            Err(error) => response(403, error),
        }
    }
}

fn filter_event(input: &[u8], authorization: Option<&str>) -> Result<Vec<u8>, String> {
    let token = authorization
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| "missing Biscuit bearer token".to_owned())?;
    if token.trim().is_empty() {
        return Err("empty Biscuit bearer token".to_owned());
    }
    let event: MutationEvent = serde_json::from_slice(input)
        .map_err(|error| format!("invalid mutation event: {error}"))?;
    serde_json::to_vec(&event).map_err(|error| format!("failed to encode mutation event: {error}"))
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn response(
    status: u16,
    body: impl Into<Vec<u8>>,
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body: body.into(),
        trailers: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_biscuit_bearer_before_forwarding() {
        assert!(filter_event(br#"{"namespace":"n","key":"k","op":"insert"}"#, None).is_err());
        assert!(filter_event(
            br#"{"namespace":"n","key":"k","op":"insert"}"#,
            Some("Bearer token")
        )
        .is_ok());
    }
}
