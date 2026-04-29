mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde_json::{json, Value};
use std::{fs::OpenOptions, io::Write};

const METERING_OUTPUT_PATH: &str = "/app/data/metering.ndjson";

struct Component;

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        if !req.method.eq_ignore_ascii_case("POST") {
            return response(405, "Method Not Allowed");
        }

        match append_metering_record(METERING_OUTPUT_PATH, &req.body) {
            Ok(()) => response(202, "Accepted"),
            Err(error) => response(500, error),
        }
    }
}

fn append_metering_record(path: &str, body: &[u8]) -> Result<(), String> {
    let body = normalize_metering_batch(body)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("failed to open `{path}`: {error}"))?;
    file.write_all(&body)
        .map_err(|error| format!("failed to append metering batch: {error}"))?;
    if !body.ends_with(b"\n") {
        file.write_all(b"\n")
            .map_err(|error| format!("failed to terminate metering batch: {error}"))?;
    }
    file.flush()
        .map_err(|error| format!("failed to flush metering batch: {error}"))?;
    Ok(())
}

fn normalize_metering_batch(body: &[u8]) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(body)
        .map_err(|error| format!("metering payload is not UTF-8: {error}"))?;
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        lines.push(normalize_metering_line(trimmed)?);
    }
    if lines.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = lines.join("\n").into_bytes();
    out.push(b'\n');
    Ok(out)
}

fn normalize_metering_line(line: &str) -> Result<String, String> {
    let Ok(mut value) = serde_json::from_str::<Value>(line) else {
        return Ok(line.to_owned());
    };
    if value.get("event").and_then(Value::as_str) == Some("lora_adapter_load") {
        let overhead_us = value
            .get("overhead_us")
            .and_then(Value::as_u64)
            .or_else(|| value.get("duration_us").and_then(Value::as_u64))
            .unwrap_or_default();
        let object = value
            .as_object_mut()
            .ok_or_else(|| "LoRA metering event must be a JSON object".to_owned())?;
        object.insert("meter_kind".to_owned(), json!("fuel"));
        object.insert("fuel_consumed".to_owned(), json!(overhead_us));
    }
    serde_json::to_string(&value)
        .map_err(|error| format!("failed to encode metering record: {error}"))
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
    fn append_metering_record_adds_trailing_newline() {
        let path = unique_test_file("tachyon-metering");

        append_metering_record(
            path.to_str().expect("path should be utf-8"),
            br#"{"trace_id":"abc"}"#,
        )
        .expect("record should append");

        assert_eq!(
            fs::read_to_string(&path).expect("metering file should be readable"),
            "{\"trace_id\":\"abc\"}\n"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn append_metering_record_preserves_existing_newline_delimited_batches() {
        let path = unique_test_file("tachyon-metering-batch");

        append_metering_record(
            path.to_str().expect("path should be utf-8"),
            b"{\"trace_id\":\"a\"}\n{\"trace_id\":\"b\"}\n",
        )
        .expect("batch should append");
        append_metering_record(
            path.to_str().expect("path should be utf-8"),
            b"{\"trace_id\":\"c\"}",
        )
        .expect("second batch should append");

        assert_eq!(
            fs::read_to_string(&path).expect("metering file should be readable"),
            "{\"trace_id\":\"a\"}\n{\"trace_id\":\"b\"}\n{\"trace_id\":\"c\"}\n"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn lora_adapter_load_records_fuel_consumption() {
        let path = unique_test_file("tachyon-metering-lora");

        append_metering_record(
            path.to_str().expect("path should be utf-8"),
            br#"{"event":"lora_adapter_load","adapter_id":"tenant-a","overhead_us":42}"#,
        )
        .expect("LoRA record should append");

        let record: serde_json::Value = serde_json::from_str(
            fs::read_to_string(&path)
                .expect("metering file should be readable")
                .trim(),
        )
        .expect("record should parse");
        assert_eq!(record["meter_kind"], "fuel");
        assert_eq!(record["fuel_consumed"], 42);

        let _ = fs::remove_file(path);
    }
}
