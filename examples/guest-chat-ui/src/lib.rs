mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "faas-guest",
    });

    export!(Component);
}

const INDEX_HTML: &str = include_str!("../assets/index.html");
const CHAT_COMPONENT_JS: &str = include_str!("../assets/tachyon-chat-assistant.js");
const STYLES_CSS: &str = include_str!("../assets/styles.css");
const CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

struct Component;

#[derive(Debug, PartialEq, Eq)]
struct StaticResponse {
    status: u16,
    content_type: &'static str,
    body: &'static [u8],
    etag: &'static str,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let request = StaticRequest {
            method: &req.method,
            uri: &req.uri,
            headers: req
                .headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.as_str()))
                .collect(),
        };
        let response = route_request(&request);
        to_guest_response(response)
    }
}

struct StaticRequest<'a> {
    method: &'a str,
    uri: &'a str,
    headers: Vec<(&'a str, &'a str)>,
}

fn route_request(request: &StaticRequest<'_>) -> StaticResponse {
    if !request.method.eq_ignore_ascii_case("GET") && !request.method.eq_ignore_ascii_case("HEAD") {
        return static_response(
            405,
            "text/plain; charset=utf-8",
            b"method not allowed",
            "\"tachyon-chat-ui-error\"",
        );
    }

    let path = route_path(request.uri);
    let asset = match path {
        "/chat" | "/chat/" | "/chat/index.html" => static_response(
            200,
            "text/html; charset=utf-8",
            INDEX_HTML.as_bytes(),
            "\"tachyon-chat-ui-index-v1\"",
        ),
        "/chat/tachyon-chat-assistant.js" => static_response(
            200,
            "application/javascript; charset=utf-8",
            CHAT_COMPONENT_JS.as_bytes(),
            "\"tachyon-chat-ui-component-v1\"",
        ),
        "/chat/styles.css" => static_response(
            200,
            "text/css; charset=utf-8",
            STYLES_CSS.as_bytes(),
            "\"tachyon-chat-ui-styles-v1\"",
        ),
        _ => static_response(
            404,
            "text/plain; charset=utf-8",
            b"asset not found",
            "\"tachyon-chat-ui-missing\"",
        ),
    };

    if asset.status == 200 && request_etag_matches(&request.headers, asset.etag) {
        static_response(304, asset.content_type, b"", asset.etag)
    } else if request.method.eq_ignore_ascii_case("HEAD") {
        static_response(asset.status, asset.content_type, b"", asset.etag)
    } else {
        asset
    }
}

fn static_response(
    status: u16,
    content_type: &'static str,
    body: &'static [u8],
    etag: &'static str,
) -> StaticResponse {
    StaticResponse {
        status,
        content_type,
        body,
        etag,
    }
}

fn to_guest_response(
    response: StaticResponse,
) -> bindings::exports::tachyon::mesh::handler::Response {
    let cache_control = if matches!(response.status, 200 | 304) {
        CACHE_CONTROL
    } else {
        "no-store"
    };
    bindings::exports::tachyon::mesh::handler::Response {
        status: response.status,
        headers: vec![
            ("content-type".to_owned(), response.content_type.to_owned()),
            ("cache-control".to_owned(), cache_control.to_owned()),
            ("etag".to_owned(), response.etag.to_owned()),
        ],
        body: response.body.to_vec(),
        trailers: vec![],
    }
}

fn route_path(uri: &str) -> &str {
    uri.split_once('?').map(|(path, _)| path).unwrap_or(uri)
}

fn request_etag_matches(headers: &[(&str, &str)], expected: &str) -> bool {
    headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("if-none-match") && etag_list_matches(value, expected)
    })
}

fn etag_list_matches(value: &str, expected: &str) -> bool {
    value
        .split(',')
        .map(str::trim)
        .any(|candidate| candidate == "*" || candidate == expected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request<'a>(
        method: &'a str,
        uri: &'a str,
        headers: Vec<(&'a str, &'a str)>,
    ) -> StaticRequest<'a> {
        StaticRequest {
            method,
            uri,
            headers,
        }
    }

    #[test]
    fn serves_index_for_chat_root() {
        let response = route_request(&request("GET", "/chat", vec![]));

        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "text/html; charset=utf-8");
        assert!(std::str::from_utf8(response.body)
            .expect("index is utf-8")
            .contains("tachyon-chat-assistant"));
    }

    #[test]
    fn serves_web_component_script() {
        let response = route_request(&request("GET", "/chat/tachyon-chat-assistant.js", vec![]));

        assert_eq!(response.status, 200);
        assert_eq!(
            response.content_type,
            "application/javascript; charset=utf-8"
        );
        assert!(std::str::from_utf8(response.body)
            .expect("script is utf-8")
            .contains("customElements.define"));
    }

    #[test]
    fn validates_etag_and_returns_not_modified() {
        let response = route_request(&request(
            "GET",
            "/chat/tachyon-chat-assistant.js",
            vec![("If-None-Match", "\"tachyon-chat-ui-component-v1\"")],
        ));

        assert_eq!(response.status, 304);
        assert!(response.body.is_empty());
    }

    #[test]
    fn head_returns_headers_without_body() {
        let response = route_request(&request("HEAD", "/chat/styles.css", vec![]));

        assert_eq!(response.status, 200);
        assert_eq!(response.content_type, "text/css; charset=utf-8");
        assert!(response.body.is_empty());
    }

    #[test]
    fn rejects_unknown_assets() {
        let response = route_request(&request("GET", "/chat/unknown.js", vec![]));

        assert_eq!(response.status, 404);
    }
}
