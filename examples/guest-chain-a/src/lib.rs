mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

struct Component;

/// Entry hop of the `bench/` 3-FaaS chain scenario (see
/// `bench/workloads/faas-chain.yaml`): forwards to `guest-chain-b`, which
/// forwards to `guest-chain-c`, exercising a 3-hop in-process mesh dispatch
/// chain (#288-#296). Setting `TACHYON_BENCH_FORCE_MESH_TRANSPORT=1` on the
/// core-host process makes every hop fall back to the UDS/TCP transport
/// instead, for an apples-to-apples comparison.
impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        match bindings::tachyon::mesh::outbound_http::send_request(
            "GET",
            "http://tachyon/guest-chain-b",
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
                body: format!("guest-chain-a forward to guest-chain-b failed: {error}")
                    .into_bytes(),
                trailers: vec![],
            },
        }
    }
}
