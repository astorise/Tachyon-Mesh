mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../wit",
        world: "faas-guest",
    });

    export!(Component);
}

struct Component;

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let body = if req.body.is_empty() {
            b"FaaS received an empty payload".to_vec()
        } else {
            format!("FaaS received: {}", String::from_utf8_lossy(&req.body)).into_bytes()
        };

        bindings::exports::tachyon::mesh::handler::Response { status: 200, body }
    }
}
