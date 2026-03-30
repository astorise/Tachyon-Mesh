mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

const LEGACY_ROUTE: &str = "/api/guest-call-legacy";

struct Component;

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        _req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let pending =
            bindings::tachyon::mesh::scaling_metrics::get_pending_queue_size(LEGACY_ROUTE);
        let body = format!(
            concat!(
                "# TYPE tachyon_pending_requests gauge\n",
                "tachyon_pending_requests{{route=\"{}\"}} {}\n",
            ),
            LEGACY_ROUTE, pending
        )
        .into_bytes();

        bindings::exports::tachyon::mesh::handler::Response { status: 200, body }
    }
}
