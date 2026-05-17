mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};

const BACKGROUND_PRIORITY: &str = "background";

struct Component;

#[derive(Debug, Deserialize)]
struct MutationEvent {
    key: String,
    #[serde(default)]
    vector_clock: u64,
}

#[derive(Debug, Deserialize, Serialize, PartialEq)]
struct MaterializedView {
    key: String,
    vector_clock: u64,
    priority: String,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        match rebuild_view(&req.body, 0) {
            Ok(body) => response(202, body),
            Err(error) => response(400, error),
        }
    }
}

fn rebuild_view(input: &[u8], current_vector_clock: u64) -> Result<Vec<u8>, String> {
    let event: MutationEvent = serde_json::from_slice(input)
        .map_err(|error| format!("invalid mutation event: {error}"))?;
    if event.vector_clock < current_vector_clock {
        return Err("stale CDC event ignored".to_owned());
    }
    let view = MaterializedView {
        key: format!("V:{}", event.key),
        vector_clock: event.vector_clock,
        priority: BACKGROUND_PRIORITY.to_owned(),
    };
    serde_json::to_vec(&view)
        .map_err(|error| format!("failed to encode materialized view: {error}"))
}

fn response(
    status: u16,
    body: impl Into<Vec<u8>>,
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body: body.into(),
        trailers: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuilds_view_with_vector_clock() {
        let view: MaterializedView = serde_json::from_slice(
            &rebuild_view(br#"{"key":"dashboard:u1","vector_clock":7}"#, 3).unwrap(),
        )
        .unwrap();

        assert_eq!(
            view,
            MaterializedView {
                key: "V:dashboard:u1".to_owned(),
                vector_clock: 7,
                priority: "background".to_owned(),
            }
        );
        assert!(rebuild_view(br#"{"key":"dashboard:u1","vector_clock":1}"#, 3).is_err());
    }
}
