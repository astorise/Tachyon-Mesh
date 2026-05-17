mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

const RANGE_HEADER: &str = "range";

struct Component;

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let Some(range) = header_value(&req.headers, RANGE_HEADER) else {
            return response(416, "missing Range header", &[]);
        };
        let Ok((start, end)) = parse_range_header(range) else {
            return response(416, "invalid Range header", &[]);
        };
        let body = format!("media range {start}-{end} queued for zero-copy host pipe");
        response(
            206,
            body,
            &[
                ("accept-ranges", "bytes"),
                ("content-range", &format!("bytes {start}-{end}/*")),
            ],
        )
    }
}

fn parse_range_header(value: &str) -> Result<(u64, u64), String> {
    let value = value
        .trim()
        .strip_prefix("bytes=")
        .ok_or_else(|| "range must start with bytes=".to_owned())?;
    let (start, end) = value
        .split_once('-')
        .ok_or_else(|| "range must contain '-'".to_owned())?;
    let start = start
        .trim()
        .parse::<u64>()
        .map_err(|_| "invalid range start".to_owned())?;
    let end = end
        .trim()
        .parse::<u64>()
        .map_err(|_| "invalid range end".to_owned())?;
    if end < start {
        return Err("range end before start".to_owned());
    }
    Ok((start, end))
}

fn header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_name, _)| header_name.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn response(
    status: u16,
    body: impl Into<Vec<u8>>,
    headers: &[(&str, &str)],
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        body: body.into(),
        trailers: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_byte_range() {
        assert_eq!(parse_range_header("bytes=10-42").unwrap(), (10, 42));
        assert!(parse_range_header("bytes=42-10").is_err());
    }
}
