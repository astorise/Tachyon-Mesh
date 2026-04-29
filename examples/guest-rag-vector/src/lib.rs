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
        _req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let spec = bindings::tachyon::mesh::vector::IndexSpec {
            name: "tenant-kb".to_owned(),
            dim: 3,
            m: 16,
            ef_construction: 200,
        };
        let result = bindings::tachyon::mesh::vector::create_index(&spec)
            .and_then(|_| {
                bindings::tachyon::mesh::vector::upsert(
                    "tenant-kb",
                    &[bindings::tachyon::mesh::vector::Document {
                        id: "doc-edge".to_owned(),
                        embedding: vec![1.0, 0.0, 0.0],
                        payload: Some(b"edge private context".to_vec()),
                    }],
                )
            })
            .and_then(|_| {
                bindings::tachyon::mesh::vector::search("tenant-kb", &[0.9, 0.1, 0.0], 1)
            });

        match result {
            Ok(matches) => response(
                200,
                matches
                    .first()
                    .and_then(|item| item.payload.as_ref())
                    .cloned()
                    .unwrap_or_else(|| b"no match".to_vec()),
            ),
            Err(error) => response(500, error.into_bytes()),
        }
    }
}

fn response(status: u16, body: Vec<u8>) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: Vec::new(),
        body,
        trailers: Vec::new(),
    }
}
