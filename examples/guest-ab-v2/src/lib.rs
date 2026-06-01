include!(concat!(env!("OUT_DIR"), "/version_section.rs"));

mod bindings {
    use super::Component;
    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "faas-guest",
    });
    export!(Component);
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Component;

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let name = extract_name(&req);
        let request_id = extract_header(&req, "x-request-id")
            .unwrap_or_else(|| "unknown".to_owned());
        let body = format!(
            r#"{{"version":"{VERSION}","variant":"canary","greeting":"Hello, {name}!","algorithm":"personalized","request_id":"{request_id}","features":["enhanced-routing","request-tracing"]}}"#
        )
        .into_bytes();

        bindings::exports::tachyon::mesh::handler::Response {
            status: 200,
            headers: vec![
                ("content-type".to_owned(), b"application/json".to_vec()),
                ("x-faas-version".to_owned(), VERSION.as_bytes().to_vec()),
                ("x-faas-variant".to_owned(), b"canary".to_vec()),
            ],
            body,
            trailers: vec![],
        }
    }
}

fn extract_name(req: &bindings::exports::tachyon::mesh::handler::Request) -> String {
    extract_header(req, "x-guest-name")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "World".to_owned())
}

fn extract_header(
    req: &bindings::exports::tachyon::mesh::handler::Request,
    name: &str,
) -> Option<String> {
    req.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .and_then(|(_, v)| String::from_utf8(v.clone()).ok())
}
