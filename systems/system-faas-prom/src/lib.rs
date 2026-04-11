mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

struct Component;

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        _req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let metrics = bindings::tachyon::mesh::telemetry_reader::get_metrics();
        let body = format!(
            concat!(
                "# TYPE tachyon_requests_total counter\n",
                "tachyon_requests_total {}\n",
                "# TYPE tachyon_requests_completed_total counter\n",
                "tachyon_requests_completed_total {}\n",
                "# TYPE tachyon_requests_error_total counter\n",
                "tachyon_requests_error_total {}\n",
                "# TYPE tachyon_active_requests gauge\n",
                "tachyon_active_requests {}\n",
                "# TYPE tachyon_telemetry_dropped_events_total counter\n",
                "tachyon_telemetry_dropped_events_total {}\n",
                "# TYPE tachyon_last_status gauge\n",
                "tachyon_last_status {}\n",
                "# TYPE tachyon_total_duration_us_total counter\n",
                "tachyon_total_duration_us_total {}\n",
                "# TYPE tachyon_total_wasm_duration_us_total counter\n",
                "tachyon_total_wasm_duration_us_total {}\n",
                "# TYPE tachyon_total_host_overhead_us_total counter\n",
                "tachyon_total_host_overhead_us_total {}\n",
            ),
            metrics.total_requests,
            metrics.completed_requests,
            metrics.error_requests,
            metrics.active_requests,
            metrics.dropped_events,
            metrics.last_status,
            metrics.total_duration_us,
            metrics.total_wasm_duration_us,
            metrics.total_host_overhead_us,
        )
        .into_bytes();

        bindings::exports::tachyon::mesh::handler::Response {
            status: 200,
            headers: vec![],
            body,
            trailers: vec![],
        }
    }
}
