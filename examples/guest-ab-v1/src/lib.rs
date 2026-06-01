// Version embedded as a WASM custom section ("tachyon.version") at compile
// time via build.rs — stays in sync with Cargo.toml automatically.
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
        let body = format!(
            r#"{{"version":"{VERSION}","variant":"stable","greeting":"Hello, {name}!","algorithm":"round-robin"}}"#
        )
        .into_bytes();

        bindings::exports::tachyon::mesh::handler::Response {
            status: 200,
            headers: vec![
                ("content-type".to_owned(), "application/json".to_owned()),
                ("x-faas-version".to_owned(), VERSION.to_owned()),
            ],
            body,
            trailers: vec![],
        }
    }
}

fn extract_name(req: &bindings::exports::tachyon::mesh::handler::Request) -> String {
    req.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("x-guest-name"))
        .map(|(_, v)| v.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "World".to_owned())
}
