use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "faas-guest",
    });

    export!(Component);
}

struct Component;

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let body = String::from_utf8_lossy(&req.body);

        if let Some(delay_ms) = parse_delay_ms(&body) {
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(delay_ms.min(5_000)) {
                std::hint::spin_loop();
            }
            return response(200, format!("slept:{delay_ms}"));
        }

        if body.contains("force-ok") {
            return response(200, "MOCK_OK".to_owned());
        }

        if body.contains("force-fail") {
            return response(503, "MOCK_UNAVAILABLE".to_owned());
        }

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.subsec_nanos())
            .unwrap_or_default();
        if nanos % 10 < 7 {
            response(503, "MOCK_UNAVAILABLE".to_owned())
        } else {
            response(200, "MOCK_OK".to_owned())
        }
    }
}

fn parse_delay_ms(body: &str) -> Option<u64> {
    body.trim()
        .strip_prefix("sleep:")
        .and_then(|value| value.parse::<u64>().ok())
}

fn response(status: u16, body: String) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: vec![],
        body: body.into_bytes(),
        trailers: vec![],
    }
}
