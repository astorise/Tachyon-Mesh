mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::Deserialize;

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
}
