mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use bindings::tachyon::mesh::storage_broker::WriteMode;

struct Component;

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        if !req.method.eq_ignore_ascii_case("POST") {
            return response(405, "Method Not Allowed");
        }

        let (path, mode) = match parse_write_request(&req.uri) {
            Ok(parsed) => parsed,
            Err(message) => return response(400, message),
        };

        match bindings::tachyon::mesh::storage_broker::enqueue_write(&path, mode, &req.body) {
            Ok(()) => response(202, "Accepted"),
            Err(error) => response(500, error),
        }
    }
}

fn parse_write_request(uri: &str) -> Result<(String, WriteMode), &'static str> {
    let query = uri
        .split_once('?')
        .map(|(_, query)| query)
        .ok_or("storage broker requests must include a `path` query parameter")?;
    let mut path = None;
    let mut mode = WriteMode::Overwrite;

    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "path" => path = Some(value.to_owned()),
            "mode" if value.eq_ignore_ascii_case("overwrite") => mode = WriteMode::Overwrite,
            "mode" if value.eq_ignore_ascii_case("append") => mode = WriteMode::Append,
            "mode" => return Err("storage broker `mode` must be `overwrite` or `append`"),
            _ => {}
        }
    }

    let path = path.ok_or("storage broker requests must include a `path` query parameter")?;
    if path.trim().is_empty() {
        return Err("storage broker `path` must not be empty");
    }

    Ok((path, mode))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_write_request_defaults_to_overwrite_mode() {
        let (path, mode) = parse_write_request("/system/storage-broker?path=/app/data/state.txt")
            .expect("request should parse");

        assert_eq!(path, "/app/data/state.txt");
        assert_eq!(mode, WriteMode::Overwrite);
    }

    #[test]
    fn parse_write_request_accepts_append_mode() {
        let (_, mode) =
            parse_write_request("/system/storage-broker?path=/app/data/state.txt&mode=append")
                .expect("request should parse");

        assert_eq!(mode, WriteMode::Append);
    }

    #[test]
    fn parse_write_request_rejects_unknown_mode() {
        let error =
            parse_write_request("/system/storage-broker?path=/app/data/state.txt&mode=merge")
                .expect_err("invalid mode should fail");

        assert_eq!(
            error,
            "storage broker `mode` must be `overwrite` or `append`"
        );
    }
}
