mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

struct Component;

/// Middle hop of the `bench/` 3-FaaS chain scenario: forwards to
/// `guest-chain-c` via the internal mesh dispatch path so the bench harness
/// can measure a 3-hop in-process chain end to end.
impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        match bindings::tachyon::mesh::outbound_http::send_request(
            "GET",
            "http://tachyon/guest-chain-c",
            &[],
            &req.body,
        ) {
            Ok(forwarded) => bindings::exports::tachyon::mesh::handler::Response {
                status: forwarded.status,
                headers: forwarded.headers,
                body: forwarded.body,
                trailers: vec![],
            },
            Err(error) => bindings::exports::tachyon::mesh::handler::Response {
                status: 502,
                headers: vec![],
                body: format!("guest-chain-b forward to guest-chain-c failed: {error}")
                    .into_bytes(),
                trailers: vec![],
            },
        }
    }
}
