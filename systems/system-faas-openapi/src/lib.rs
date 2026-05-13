mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use bindings::exports::tachyon::mesh::handler::{Guest, Request, Response};
use bindings::tachyon::mesh::outbound_http;

static SWAGGER_UI_HTML: &str = include_str!("swagger-ui.html");

pub struct Component;

impl Guest for Component {
    fn handle_request(req: Request) -> Response {
        let path = req.uri.split('?').next().unwrap_or(&req.uri);

        match path {
            "/admin/docs" => Response {
                status: 200,
                headers: vec![(
                    "content-type".to_string(),
                    "text/html; charset=utf-8".to_string(),
                )],
                body: SWAGGER_UI_HTML.as_bytes().to_vec(),
                trailers: vec![],
            },
            "/admin/schema/openapi.json" => {
                // Proxy the schema from the core-host's own endpoint at a well-known
                // loopback address. The host serves the utoipa-generated OpenAPI JSON.
                match outbound_http::send_request(
                    "GET",
                    "http://127.0.0.1:8080/admin/schema/openapi.json",
                    &req.headers,
                    &[],
                ) {
                    Ok(resp) => Response {
                        status: resp.status,
                        headers: resp.headers,
                        body: resp.body,
                        trailers: vec![],
                    },
                    Err(e) => Response {
                        status: 502,
                        headers: vec![("content-type".to_string(), "text/plain".to_string())],
                        body: format!("upstream error: {e}").into_bytes(),
                        trailers: vec![],
                    },
                }
            }
            _ => Response {
                status: 404,
                headers: vec![("content-type".to_string(), "text/plain".to_string())],
                body: b"not found".to_vec(),
                trailers: vec![],
            },
        }
    }
}
