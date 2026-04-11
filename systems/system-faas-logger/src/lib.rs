mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};
use std::{fs::OpenOptions, io::Write};

const LOGGER_OUTPUT_PATH: &str = "/app/data/guest-logs.ndjson";

#[derive(Debug, Deserialize, Serialize)]
struct LogEntry {
    target_name: String,
    timestamp_unix_ms: u64,
    stream_type: String,
    level: String,
    message: String,
    #[serde(default)]
    guest_target: Option<String>,
    #[serde(default)]
    structured_fields: Option<serde_json::Value>,
}

struct Component;

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        if !req.method.eq_ignore_ascii_case("POST") {
            return response(405, "Method Not Allowed");
        }

        let entries = match serde_json::from_slice::<Vec<LogEntry>>(&req.body) {
            Ok(entries) => entries,
            Err(error) => return response(400, format!("invalid log batch payload: {error}")),
        };

        match append_log_batch(LOGGER_OUTPUT_PATH, &entries) {
            Ok(()) => response(202, "Accepted"),
            Err(error) => response(500, error),
        }
    }
}

fn append_log_batch(path: &str, entries: &[LogEntry]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("failed to open `{path}`: {error}"))?;

    for entry in entries {
        let encoded = serde_json::to_vec(entry)
            .map_err(|error| format!("failed to encode log entry: {error}"))?;
        file.write_all(&encoded)
            .map_err(|error| format!("failed to append log entry: {error}"))?;
        file.write_all(b"\n")
            .map_err(|error| format!("failed to terminate log entry: {error}"))?;
    }

    file.flush()
        .map_err(|error| format!("failed to flush logger batch: {error}"))?;
    Ok(())
}

fn response(
    status: u16,
    body: impl Into<Vec<u8>>,
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: vec![],
        body: body.into(),
        trailers: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_test_file(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}.ndjson"))
    }

    #[test]
    fn append_log_batch_writes_ndjson_records() {
        let path = unique_test_file("tachyon-async-logger");
        let entries = vec![LogEntry {
            target_name: "guest-log-storm".to_owned(),
            timestamp_unix_ms: 1,
            stream_type: "stdout".to_owned(),
            level: "info".to_owned(),
            message: "storm-1".to_owned(),
            guest_target: Some("guest-log-storm".to_owned()),
            structured_fields: None,
        }];

        append_log_batch(path.to_str().expect("path should be utf-8"), &entries)
            .expect("batch should append");

        let contents = fs::read_to_string(&path).expect("logger file should be readable");
        assert!(contents.contains("\"message\":\"storm-1\""));

        let _ = fs::remove_file(path);
    }
}
