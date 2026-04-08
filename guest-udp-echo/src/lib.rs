use std::time::{Duration, Instant};

mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../wit",
        world: "udp-faas-guest",
    });

    export!(Component);
}

struct Component;

impl bindings::exports::tachyon::mesh::udp_handler::Guest for Component {
    fn handle_packet(
        source_ip: String,
        source_port: u16,
        payload: Vec<u8>,
    ) -> Vec<bindings::exports::tachyon::mesh::udp_handler::Datagram> {
        if let Some(delay_ms) = parse_delay_ms(&payload) {
            let start = Instant::now();
            while start.elapsed() < Duration::from_millis(delay_ms) {
                std::hint::spin_loop();
            }
        }

        vec![bindings::exports::tachyon::mesh::udp_handler::Datagram {
            target_ip: source_ip,
            target_port: source_port,
            payload,
        }]
    }
}

fn parse_delay_ms(payload: &[u8]) -> Option<u64> {
    let value = std::str::from_utf8(payload).ok()?;
    let delay_ms = value.trim().strip_prefix("delay:")?.parse::<u64>().ok()?;
    Some(delay_ms.min(1_000))
}
