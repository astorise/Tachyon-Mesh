mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

const ORIGINAL_ROUTE_HEADER: &str = "x-tachyon-original-route";
const GATEWAY_ROUTE: &str = "/system/gateway";

struct Component;

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let Some(original_route) = req
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(ORIGINAL_ROUTE_HEADER))
            .map(|(_, value)| value.trim().to_owned())
        else {
            return response(400, "missing x-tachyon-original-route header");
        };

        let target_route = match normalize_original_route(&original_route) {
            Ok(route) => route,
            Err(error) => return response(400, error),
        };

        let forward_headers = forwarded_headers(&req.headers);
        match bindings::tachyon::mesh::outbound_http::send_request(
            &req.method,
            &format!("http://mesh{target_route}"),
            &forward_headers,
            &req.body,
        ) {
            Ok(forwarded) => bindings::exports::tachyon::mesh::handler::Response {
                status: forwarded.status,
                headers: forwarded.headers,
                body: forwarded.body,
                trailers: vec![],
            },
            Err(error) => response(502, format!("gateway forward failed: {error}")),
        }
    }
}

fn normalize_original_route(route: &str) -> Result<String, String> {
    let trimmed = route.trim();
    if trimmed.is_empty() {
        return Err("original route must not be empty".to_owned());
    }
    let normalized = if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    };
    if normalized == GATEWAY_ROUTE || normalized.starts_with("/system/gateway?") {
        return Err("gateway route cannot forward to itself".to_owned());
    }
    Ok(normalized)
}

fn forwarded_headers(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, _)| {
            !name.eq_ignore_ascii_case(ORIGINAL_ROUTE_HEADER)
                && !name.eq_ignore_ascii_case("host")
                && !name.eq_ignore_ascii_case("connection")
                && !name.eq_ignore_ascii_case("content-length")
        })
        .cloned()
        .collect()
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
    fn normalize_original_route_adds_leading_slash() {
        assert_eq!(
            normalize_original_route("api/demo").expect("route should normalize"),
            "/api/demo"
        );
    }

    #[test]
    fn normalize_original_route_rejects_gateway_loop() {
        assert!(normalize_original_route("/system/gateway").is_err());
    }

    #[test]
    fn forwarded_headers_strip_gateway_internal_headers() {
        let headers = forwarded_headers(&[
            ("host".to_owned(), "example.com".to_owned()),
            (
                "x-tachyon-original-route".to_owned(),
                "/api/demo".to_owned(),
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
        ]);
        assert_eq!(
            headers,
            vec![("content-type".to_owned(), "application/json".to_owned())]
        );
    }
}
