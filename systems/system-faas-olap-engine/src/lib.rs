mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::Deserialize;

const MAX_PAYLOAD_BYTES: usize = 2 * 1024 * 1024;

struct Component;

#[derive(Debug, Deserialize)]
struct Row {
    #[serde(default)]
    group: String,
    #[serde(default)]
    value: f64,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        match aggregate_sum_by_group(&req.body) {
            Ok(body) => response(200, body),
            Err(error) => response(400, error),
        }
    }
}

fn aggregate_sum_by_group(input: &[u8]) -> Result<Vec<u8>, String> {
    // Bound the JSON payload before deserialization to prevent memory amplification
    // attacks via deeply nested or massive arrays.
    if input.len() > MAX_PAYLOAD_BYTES {
        return Err(format!(
            "OLAP payload exceeds {MAX_PAYLOAD_BYTES} byte limit",
        ));
    }
    let rows: Vec<Row> =
        serde_json::from_slice(input).map_err(|error| format!("invalid OLAP rows: {error}"))?;
    let mut groups = std::collections::BTreeMap::<String, f64>::new();
    for row in rows {
        *groups.entry(row.group).or_default() += row.value;
    }
    serde_json::to_vec(&groups).map_err(|error| format!("failed to encode OLAP result: {error}"))
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
    fn aggregates_rows_by_group() {
        let result = aggregate_sum_by_group(
            br#"[{"group":"a","value":2.5},{"group":"a","value":1.5},{"group":"b","value":4}]"#,
        )
        .unwrap();

        assert_eq!(
            std::str::from_utf8(&result).unwrap(),
            r#"{"a":4.0,"b":4.0}"#
        );
    }

    #[test]
    fn rejects_oversized_payloads() {
        let body = vec![b'a'; MAX_PAYLOAD_BYTES + 1];
        let response =
            aggregate_sum_by_group(&body).expect_err("oversized payload should be refused");
        assert!(response.contains("byte limit"));
    }
}
