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

#[derive(Debug, PartialEq, Eq)]
enum BrokerOperation {
    Write {
        path: String,
        mode: WriteMode,
    },
    Snapshot {
        volume_id: String,
        source_path: String,
        snapshot_path: String,
    },
    Restore {
        volume_id: String,
        snapshot_path: String,
        destination_path: String,
    },
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        if !req.method.eq_ignore_ascii_case("POST") {
            return response(405, "Method Not Allowed");
        }

        let operation = match parse_broker_request(&req.uri) {
            Ok(parsed) => parsed,
            Err(message) => return response(400, message),
        };

        let result = match operation {
            BrokerOperation::Write { path, mode } => {
                bindings::tachyon::mesh::storage_broker::enqueue_write(&path, mode, &req.body)
            }
            BrokerOperation::Snapshot {
                volume_id,
                source_path,
                snapshot_path,
            } => bindings::tachyon::mesh::storage_broker::snapshot_volume(
                &volume_id,
                &source_path,
                &snapshot_path,
            ),
            BrokerOperation::Restore {
                volume_id,
                snapshot_path,
                destination_path,
            } => bindings::tachyon::mesh::storage_broker::restore_volume(
                &volume_id,
                &snapshot_path,
                &destination_path,
            ),
        };

        match result {
            Ok(()) => response(202, "Accepted"),
            Err(error) => {
                let (status, body) = map_broker_error(error);
                response(status, body)
            }
        }
    }
}

fn map_broker_error(error: String) -> (u16, Vec<u8>) {
    if let Some(message) = error.strip_prefix("forbidden:") {
        return (403, message.trim().as_bytes().to_vec());
    }

    (500, error.into_bytes())
}

fn parse_broker_request(uri: &str) -> Result<BrokerOperation, &'static str> {
    let query = uri
        .split_once('?')
        .map(|(_, query)| query)
        .ok_or("storage broker requests must include query parameters")?;
    let mut operation = None;
    let mut path = None;
    let mut mode = WriteMode::Overwrite;
    let mut volume_id = None;
    let mut source_path = None;
    let mut snapshot_path = None;
    let mut destination_path = None;

    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        match key {
            "op" => operation = Some(value),
            "path" => path = Some(value.to_owned()),
            "mode" if value.eq_ignore_ascii_case("overwrite") => mode = WriteMode::Overwrite,
            "mode" if value.eq_ignore_ascii_case("append") => mode = WriteMode::Append,
            "mode" => return Err("storage broker `mode` must be `overwrite` or `append`"),
            "volume_id" => volume_id = Some(value.to_owned()),
            "source_path" => source_path = Some(value.to_owned()),
            "snapshot_path" => snapshot_path = Some(value.to_owned()),
            "destination_path" => destination_path = Some(value.to_owned()),
            _ => {}
        }
    }

    match operation.unwrap_or("write") {
        "write" => {
            let path =
                path.ok_or("storage broker write requests must include a `path` query parameter")?;
            if path.trim().is_empty() {
                return Err("storage broker `path` must not be empty");
            }

            Ok(BrokerOperation::Write { path, mode })
        }
        "snapshot" => Ok(BrokerOperation::Snapshot {
            volume_id: require_query_value(volume_id, "volume_id")?,
            source_path: require_query_value(source_path, "source_path")?,
            snapshot_path: require_query_value(snapshot_path, "snapshot_path")?,
        }),
        "restore" => Ok(BrokerOperation::Restore {
            volume_id: require_query_value(volume_id, "volume_id")?,
            snapshot_path: require_query_value(snapshot_path, "snapshot_path")?,
            destination_path: require_query_value(destination_path, "destination_path")?,
        }),
        _ => Err("storage broker `op` must be `write`, `snapshot`, or `restore`"),
    }
}

fn require_query_value(value: Option<String>, key: &'static str) -> Result<String, &'static str> {
    let value = value.ok_or(match key {
        "volume_id" => "storage broker requests must include `volume_id`",
        "source_path" => "storage broker snapshot requests must include `source_path`",
        "snapshot_path" => "storage broker requests must include `snapshot_path`",
        "destination_path" => "storage broker restore requests must include `destination_path`",
        _ => "storage broker request is missing a required query parameter",
    })?;

    if value.trim().is_empty() {
        Err(match key {
            "volume_id" => "storage broker `volume_id` must not be empty",
            "source_path" => "storage broker `source_path` must not be empty",
            "snapshot_path" => "storage broker `snapshot_path` must not be empty",
            "destination_path" => "storage broker `destination_path` must not be empty",
            _ => "storage broker query parameter must not be empty",
        })
    } else {
        Ok(value)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_write_request_defaults_to_overwrite_mode() {
        let operation = parse_broker_request("/system/storage-broker?path=/app/data/state.txt")
            .expect("request should parse");

        assert_eq!(
            operation,
            BrokerOperation::Write {
                path: "/app/data/state.txt".to_owned(),
                mode: WriteMode::Overwrite,
            }
        );
    }

    #[test]
    fn parse_write_request_accepts_append_mode() {
        let operation =
            parse_broker_request("/system/storage-broker?path=/app/data/state.txt&mode=append")
                .expect("request should parse");

        assert_eq!(
            operation,
            BrokerOperation::Write {
                path: "/app/data/state.txt".to_owned(),
                mode: WriteMode::Append,
            }
        );
    }

    #[test]
    fn parse_write_request_rejects_unknown_mode() {
        let error =
            parse_broker_request("/system/storage-broker?path=/app/data/state.txt&mode=merge")
                .expect_err("invalid mode should fail");

        assert_eq!(
            error,
            "storage broker `mode` must be `overwrite` or `append`"
        );
    }

    #[test]
    fn parse_snapshot_request_extracts_required_fields() {
        let operation = parse_broker_request(
            "/system/storage-broker?op=snapshot&volume_id=cache-db&source_path=/vol/active&snapshot_path=/vol/cache.snapshot",
        )
        .expect("snapshot request should parse");

        assert_eq!(
            operation,
            BrokerOperation::Snapshot {
                volume_id: "cache-db".to_owned(),
                source_path: "/vol/active".to_owned(),
                snapshot_path: "/vol/cache.snapshot".to_owned(),
            }
        );
    }

    #[test]
    fn parse_restore_request_extracts_required_fields() {
        let operation = parse_broker_request(
            "/system/storage-broker?op=restore&volume_id=cache-db&snapshot_path=/vol/cache.snapshot&destination_path=/vol/active",
        )
        .expect("restore request should parse");

        assert_eq!(
            operation,
            BrokerOperation::Restore {
                volume_id: "cache-db".to_owned(),
                snapshot_path: "/vol/cache.snapshot".to_owned(),
                destination_path: "/vol/active".to_owned(),
            }
        );
    }
}
