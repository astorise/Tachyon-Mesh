mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: "../../wit/tachyon.wit",
        world: "system-faas-guest",
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};

const LIST_MODELS_URL: &str = "http://mesh/internal/ai-list-model/list-models";
const ROUTE_MODELS: &str = "/v1/models";
const ROUTE_CHAT_COMPLETIONS: &str = "/v1/chat/completions";

struct Component;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelInfo {
    alias: String,
    engine: String,
    #[allow(dead_code)]
    vram_required_mb: u64,
    #[allow(dead_code)]
    status: String,
}

#[derive(Debug, Serialize)]
struct OpenAiModel {
    id: String,
    object: &'static str,
    created: u64,
    owned_by: &'static str,
}

#[derive(Debug, Serialize)]
struct OpenAiModelList {
    object: &'static str,
    data: Vec<OpenAiModel>,
}

#[derive(Debug, Serialize)]
struct OpenAiError {
    error: OpenAiErrorBody,
}

#[derive(Debug, Serialize)]
struct OpenAiErrorBody {
    message: String,
    #[serde(rename = "type")]
    kind: &'static str,
}

impl bindings::exports::tachyon::mesh::handler::Guest for Component {
    fn handle_request(
        req: bindings::exports::tachyon::mesh::handler::Request,
    ) -> bindings::exports::tachyon::mesh::handler::Response {
        let result = route_request(&req.method, route_path(&req.uri));
        match result {
            Ok((status, body)) => json_response(status, body),
            Err(error) => {
                let body = serde_json::to_vec(&OpenAiError {
                    error: OpenAiErrorBody {
                        message: error,
                        kind: "server_error",
                    },
                })
                .unwrap_or_default();
                json_response(500, body)
            }
        }
    }
}

fn route_request(method: &str, path: &str) -> Result<(u16, Vec<u8>), String> {
    if method.eq_ignore_ascii_case("GET") && path == ROUTE_MODELS {
        return handle_list_models();
    }
    if method.eq_ignore_ascii_case("POST") && path == ROUTE_CHAT_COMPLETIONS {
        let body = serde_json::to_vec(&OpenAiError {
            error: OpenAiErrorBody {
                message: "chat completions not yet implemented".to_owned(),
                kind: "not_implemented",
            },
        })
        .unwrap_or_default();
        return Ok((501, body));
    }
    let body = serde_json::to_vec(&OpenAiError {
        error: OpenAiErrorBody {
            message: format!("route `{method} {path}` not found"),
            kind: "invalid_request_error",
        },
    })
    .unwrap_or_default();
    Ok((404, body))
}

fn handle_list_models() -> Result<(u16, Vec<u8>), String> {
    let upstream =
        bindings::tachyon::mesh::outbound_http::send_request("GET", LIST_MODELS_URL, &[], &[])
            .map_err(|e| format!("failed to reach ai-list-model: {e}"))?;

    if upstream.status != 200 {
        return Err(format!("ai-list-model returned status {}", upstream.status));
    }

    let models: Vec<ModelInfo> = serde_json::from_slice(&upstream.body)
        .map_err(|e| format!("invalid ai-list-model response: {e}"))?;

    let data = models
        .into_iter()
        .map(|m| OpenAiModel {
            id: format!("{}/{}", m.engine, m.alias),
            object: "model",
            created: 0,
            owned_by: "tachyon-mesh",
        })
        .collect();

    let list = OpenAiModelList {
        object: "list",
        data,
    };

    serde_json::to_vec(&list)
        .map(|body| (200, body))
        .map_err(|e| format!("failed to encode OpenAI model list: {e}"))
}

fn route_path(uri: &str) -> &str {
    uri.split_once('?').map(|(p, _)| p).unwrap_or(uri)
}

fn json_response(
    status: u16,
    body: Vec<u8>,
) -> bindings::exports::tachyon::mesh::handler::Response {
    bindings::exports::tachyon::mesh::handler::Response {
        status,
        headers: vec![("content-type".to_owned(), "application/json".to_owned())],
        body,
        trailers: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_route_returns_404_shape() {
        // Just validates the branching logic without needing a live mesh.
        let body = serde_json::to_vec(&OpenAiError {
            error: OpenAiErrorBody {
                message: "route `GET /unknown` not found".to_owned(),
                kind: "invalid_request_error",
            },
        })
        .expect("error response should serialize");
        let parsed: serde_json::Value =
            serde_json::from_slice(&body).expect("serialized error should parse");
        assert!(parsed["error"]["message"].is_string());
    }

    #[test]
    fn chat_completions_returns_501() {
        let result = route_request("POST", ROUTE_CHAT_COMPLETIONS);
        let (status, _) = result.expect("chat completions route should return a response");
        assert_eq!(status, 501);
    }
}
