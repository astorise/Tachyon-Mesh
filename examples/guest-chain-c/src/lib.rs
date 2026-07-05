mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "faas-guest",
    });

    export!(Component);
}

struct Component;

/// Terminal hop of the `bench/` 3-FaaS chain scenario (`guest-chain-a` ->
/// `guest-chain-b` -> `guest-chain-c`): echoes the request body back so the
/// bench harness can confirm the whole chain completed.
impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        bindings::exports::tachyon::mesh::handler::Response {
            status: 200,
            headers: vec![],
            body: req.body,
            trailers: vec![],
        }
    }
}
