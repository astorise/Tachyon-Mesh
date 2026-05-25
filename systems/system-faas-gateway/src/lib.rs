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
const MEDIA_SERVER_ROUTE: &str = "/system/media-server";
const OLAP_ENGINE_ROUTE: &str = "/system/olap-engine";
const MATERIALIZED_VIEW_PREFIX: &str = "/api/views/";
const OPENAI_ADAPTER_ROUTE: &str = "/system/ai-openai-adapter";
const OPENAI_V1_PREFIX: &str = "/v1/";

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

        let base_route = match normalize_original_route(&original_route) {
            Ok(route) => route,
            Err(error) => return response(400, error),
        };
        let target_route = route_baas_request(&base_route, &req.headers, &req.body);
        if target_route.kind == GatewayTargetKind::MaterializedView {
            return materialized_view_response(&base_route);
        }

        let forward_headers = forwarded_headers(&req.headers);
        match bindings::tachyon::mesh::outbound_http::send_request(
            &req.method,
            &format!("http://mesh{}", target_route.route),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GatewayTargetKind {
    Oltp,
    Media,
    Olap,
    MaterializedView,
    OpenAiAdapter,
}

struct GatewayTarget {
    route: String,
    #[allow(dead_code)]
    kind: GatewayTargetKind,
}

fn route_baas_request(
    original_route: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> GatewayTarget {
    if has_header(headers, "range") && is_media_path(original_route) {
        return GatewayTarget {
            route: MEDIA_SERVER_ROUTE.to_owned(),
            kind: GatewayTargetKind::Media,
        };
    }
    if is_olap_query(body) {
        return GatewayTarget {
            route: OLAP_ENGINE_ROUTE.to_owned(),
            kind: GatewayTargetKind::Olap,
        };
    }
    if original_route.starts_with(MATERIALIZED_VIEW_PREFIX) {
        return GatewayTarget {
            route: original_route.to_owned(),
            kind: GatewayTargetKind::MaterializedView,
        };
    }
    if original_route.starts_with(OPENAI_V1_PREFIX) || original_route == "/v1" {
        return GatewayTarget {
            route: OPENAI_ADAPTER_ROUTE.to_owned(),
            kind: GatewayTargetKind::OpenAiAdapter,
        };
    }
    GatewayTarget {
        route: original_route.to_owned(),
        kind: GatewayTargetKind::Oltp,
    }
}

fn materialized_view_key(route: &str) -> Option<String> {
    route
        .strip_prefix(MATERIALIZED_VIEW_PREFIX)
        .map(str::trim)
        .filter(|view| !view.is_empty())
        .map(|view| format!("V:{}", view.trim_matches('/')))
}

fn materialized_view_response(route: &str) -> bindings::exports::tachyon::mesh::handler::Response {
    let Some(key) = materialized_view_key(route) else {
        return response(400, "missing materialized view name");
    };
    response(200, format!("materialized view fast-path key: {key}"))
}

fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers.iter().any(|(header_name, value)| {
        header_name.eq_ignore_ascii_case(name) && !value.trim().is_empty()
    })
}

fn is_media_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.starts_with("/media/")
        || lower.ends_with(".mp4")
        || lower.ends_with(".webm")
        || lower.ends_with(".mov")
        || lower.ends_with(".m4v")
}

fn is_olap_query(body: &[u8]) -> bool {
    let Ok(query) = std::str::from_utf8(body) else {
        return false;
    };
    let upper = query.to_ascii_uppercase();
    upper.contains("GROUP BY")
        || upper.contains("SUM(")
        || upper.contains("COUNT(")
        || upper.contains("\"OLAP\"")
        || upper.contains("@AGGREGATE")
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

    #[test]
    fn range_media_routes_to_ephemeral_media_server() {
        let target = route_baas_request(
            "/media/demo.mp4",
            &[("Range".to_owned(), "bytes=0-99".to_owned())],
            b"",
        );

        assert_eq!(target.kind, GatewayTargetKind::Media);
        assert_eq!(target.route, MEDIA_SERVER_ROUTE);
    }

    #[test]
    fn aggregate_query_routes_to_olap_engine() {
        let target = route_baas_request(
            "/data/query",
            &[],
            b"select sum(total) from orders group by day",
        );

        assert_eq!(target.kind, GatewayTargetKind::Olap);
        assert_eq!(target.route, OLAP_ENGINE_ROUTE);
    }

    #[test]
    fn materialized_view_route_uses_fast_path_key() {
        let target = route_baas_request("/api/views/dashboard/user123", &[], b"");

        assert_eq!(target.kind, GatewayTargetKind::MaterializedView);
        assert_eq!(
            materialized_view_key(&target.route),
            Some("V:dashboard/user123".to_owned())
        );
    }

    #[test]
    fn v1_models_routes_to_openai_adapter() {
        let target = route_baas_request("/v1/models", &[], b"");
        assert_eq!(target.kind, GatewayTargetKind::OpenAiAdapter);
        assert_eq!(target.route, OPENAI_ADAPTER_ROUTE);
    }

    #[test]
    fn v1_chat_completions_routes_to_openai_adapter() {
        let target = route_baas_request("/v1/chat/completions", &[], b"{}");
        assert_eq!(target.kind, GatewayTargetKind::OpenAiAdapter);
        assert_eq!(target.route, OPENAI_ADAPTER_ROUTE);
    }
}
