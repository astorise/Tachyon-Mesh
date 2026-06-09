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
            Err(error) => response(401, error),
        }
    }
}

fn filter_event(input: &[u8], authorization: Option<&str>) -> Result<Vec<u8>, String> {
    // Fail-closed: the broadcaster intentionally rejects every request until a real
    // Biscuit verifier is wired in. Accepting any non-empty bearer string here was a
    // pre-production placeholder that the audit flagged as an authentication bypass.
    let _ = authorization
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| "missing Biscuit bearer token".to_owned())?;
    let _: MutationEvent = serde_json::from_slice(input)
        .map_err(|error| format!("invalid mutation event: {error}"))?;
    Err("cdc broadcaster requires a verified Biscuit token; no verifier is wired".to_owned())
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
    use crate::bindings::exports::tachyon::mesh::handler::Guest;

    #[test]
    fn rejects_missing_bearer_token() {
        assert!(filter_event(br#"{"namespace":"n","key":"k","op":"insert"}"#, None).is_err());
    }

    #[test]
    fn fail_closed_rejection_returns_unauthorized() {
        let response =
            Component::handle_request(bindings::exports::tachyon::mesh::handler::Request {
                method: "POST".to_owned(),
                uri: "/cdc".to_owned(),
                headers: vec![],
                body: br#"{"namespace":"n","key":"k","op":"insert"}"#.to_vec(),
                trailers: vec![],
            });

        assert_eq!(response.status, 401);
    }

    #[test]
    fn rejects_any_unverified_bearer_token() {
        // The previous behavior accepted any non-empty bearer string, which the
        // v1.1 audit classified as an authentication bypass. The fail-closed
        // implementation must refuse the request even when a plausible-looking
        // bearer token is presented but no Biscuit verifier is wired.
        let response = filter_event(
            br#"{"namespace":"n","key":"k","op":"insert"}"#,
            Some("Bearer placeholder"),
        );
        assert!(matches!(response, Err(ref message) if message.contains("verifier")));
    }
}
