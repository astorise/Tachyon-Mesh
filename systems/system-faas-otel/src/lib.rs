mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{fs::OpenOptions, io::Write};

const OTEL_OUTPUT_PATH: &str = "/app/data/otel-spans.ndjson";
const OTLP_ENDPOINT_HEADER: &str = "x-tachyon-otlp-endpoint";

#[derive(Debug, Deserialize, Serialize)]
struct TelemetryLine {
    trace_id: String,
    #[serde(default)]
    sampled: bool,
    #[serde(default)]
    traceparent: Option<String>,
    #[serde(default)]
    path: Option<String>,
    status: u16,
    #[serde(default)]
    fuel_consumed: Option<u64>,
    total_duration_us: u64,
    wasm_duration_us: u64,
    host_overhead_us: u64,
}

struct Component;

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        if !req.method.eq_ignore_ascii_case("POST") {
            return response(405, "Method Not Allowed");
        }

        let payload = match normalize_telemetry_batch(&req.body) {
            Ok(payload) => payload,
            Err(error) => return response(400, error),
        };

        if let Some(endpoint) = header(&req.headers, OTLP_ENDPOINT_HEADER) {
            return export_payload(endpoint, &payload);
        }

        match append_payload(OTEL_OUTPUT_PATH, &payload) {
            Ok(()) => response(202, "Accepted"),
            Err(error) => response(500, error),
        }
    }
}

fn export_payload(
    endpoint: &str,
    payload: &[u8],
) -> bindings::exports::tachyon::mesh::handler::Response {
    match bindings::tachyon::mesh::outbound_http::send_request(
        "POST",
        endpoint,
        &[
            ("content-type".to_owned(), "application/x-ndjson".to_owned()),
            (
                "x-tachyon-exporter".to_owned(),
                "system-faas-otel".to_owned(),
            ),
        ],
        payload,
    ) {
        Ok(reply) if (200..300).contains(&reply.status) => response(202, "Accepted"),
        Ok(reply) => response(502, format!("OTLP endpoint returned {}", reply.status)),
        Err(error) => response(502, format!("OTLP export failed: {error}")),
    }
}

fn normalize_telemetry_batch(body: &[u8]) -> Result<Vec<u8>, String> {
    let text = std::str::from_utf8(body)
        .map_err(|error| format!("telemetry payload is not UTF-8: {error}"))?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    if trimmed.starts_with('[') {
        let lines = serde_json::from_str::<Vec<TelemetryLine>>(trimmed)
            .map_err(|error| format!("invalid telemetry array: {error}"))?;
        return encode_spans(lines.iter());
    }

    let mut lines = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        lines.push(
            serde_json::from_str::<TelemetryLine>(line)
                .map_err(|error| format!("invalid telemetry line: {error}"))?,
        );
    }
    encode_spans(lines.iter())
}

fn encode_spans<'a>(lines: impl Iterator<Item = &'a TelemetryLine>) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for line in lines {
        let encoded = serde_json::to_vec(&otel_span(line))
            .map_err(|error| format!("failed to encode OTEL span: {error}"))?;
        out.extend_from_slice(&encoded);
        out.push(b'\n');
    }
    Ok(out)
}

fn otel_span(line: &TelemetryLine) -> Value {
    json!({
        "traceId": line.trace_id,
        "parent": line.traceparent,
        "name": line.path.as_deref().unwrap_or("tachyon.request"),
        "kind": "SERVER",
        "status": line.status,
        "attributes": {
            "tachyon.sampled": line.sampled,
            "tachyon.fuel_consumed": line.fuel_consumed,
            "tachyon.total_duration_us": line.total_duration_us,
            "tachyon.wasm_duration_us": line.wasm_duration_us,
            "tachyon.host_overhead_us": line.host_overhead_us,
        }
    })
}

fn append_payload(path: &str, payload: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|error| format!("failed to open `{path}`: {error}"))?;
    file.write_all(payload)
        .map_err(|error| format!("failed to append OTEL span batch: {error}"))?;
    file.flush()
        .map_err(|error| format!("failed to flush OTEL span batch: {error}"))
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header, _)| header.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim())
        .filter(|value| !value.is_empty())
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

    #[test]
    fn normalizes_ndjson_telemetry_to_span_records() {
        let payload = br#"{"trace_id":"abc","sampled":true,"traceparent":"00-abc","path":"/api/guest","status":200,"fuel_consumed":42,"total_duration_us":100,"wasm_duration_us":70,"host_overhead_us":30}"#;

        let normalized = normalize_telemetry_batch(payload).expect("payload should normalize");
        let value: Value = serde_json::from_slice(
            normalized
                .split(|byte| *byte == b'\n')
                .find(|line| !line.is_empty())
                .expect("one span should be emitted"),
        )
        .expect("span should parse");

        assert_eq!(value["traceId"], "abc");
        assert_eq!(value["name"], "/api/guest");
        assert_eq!(value["attributes"]["tachyon.wasm_duration_us"], 70);
    }

    #[test]
    fn normalizes_array_telemetry_to_ndjson() {
        let payload = br#"[{"trace_id":"abc","status":204,"total_duration_us":8,"wasm_duration_us":3,"host_overhead_us":5}]"#;

        let normalized = normalize_telemetry_batch(payload).expect("payload should normalize");

        assert!(String::from_utf8(normalized)
            .expect("span should be utf-8")
            .contains("\"status\":204"));
    }
}
