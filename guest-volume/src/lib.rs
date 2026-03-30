mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../wit",
        world: "faas-guest",
    });

    export!(Component);
}

const STORAGE_PATH: &str = "/app/data/state.txt";

struct Component;

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        match req.method.as_str() {
            "POST" => match std::fs::write(STORAGE_PATH, &req.body) {
                Ok(()) => response(200, "Saved"),
                Err(error) => response(500, format!("write failed: {error}")),
            },
            "GET" => match std::fs::read_to_string(STORAGE_PATH) {
                Ok(contents) => response(200, contents),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    response(200, "Empty")
                }
                Err(error) => response(500, format!("read failed: {error}")),
            },
            _ => response(405, "Method Not Allowed"),
        }
    }
}

fn response(
    status: u16,
    body: impl Into<Vec<u8>>,
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        body: body.into(),
    }
}
