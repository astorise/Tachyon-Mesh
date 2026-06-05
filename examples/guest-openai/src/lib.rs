//! `guest-openai` — a user-space FaaS that provides Tachyon's OpenAI-compatible
//! HTTP surface AND owns the AI model registry.
//!
//! It is the merger of the former `system-faas-openai-adapter` and
//! `system-faas-ai-list-model` system FaaS into a single user-role example.
//! Its `openai-faas-guest` world imports `kv-partition` (the shared
//! `ai-models-registry` table is read and written directly — no separate
//! registry FaaS and no outbound mesh hop) and `tachyon:accelerator/cpu`, so
//! `/v1/chat/completions` runs real inference on the host CPU accelerator.
//!
//! The registry is read fresh from `kv-partition` on every request (no in-guest
//! cache), so a model registered on any instance — e.g. via the
//! `system-faas-model-broker` upload notification — is immediately visible from
//! every instance. Chat completions load the named model by alias; the host
//! lazily materialises broker-uploaded models on first use.

mod bindings {
    use super::Component;

    wit_bindgen::generate!({
        path: [
            "../../wit/tachyon.wit",
            "../../wit/accelerator",
            "wit",
        ],
        world: "tachyon:openai/openai-faas-guest",
        generate_all,
    });

    export!(Component);
}

use serde::{Deserialize, Serialize};

/// Shared kv-partition table holding the model registry. The route that backs
/// this guest must declare a `scopes.kv` grant for this table name.
const MODELS_TABLE: &str = "ai-models-registry";

// Registry endpoints (internal, called by model-broker / admin tooling).
const ROUTE_REGISTER: &str = "/internal/guest-openai/register";
const ROUTE_DEREGISTER_PREFIX: &str = "/internal/guest-openai/deregister/";
// OpenAI-compatible endpoints (client-facing).
const ROUTE_MODELS: &str = "/v1/models";
const ROUTE_CHAT_COMPLETIONS: &str = "/v1/chat/completions";

/// Generation defaults when the request omits them. The host clamps these to its
/// own hard caps (`HOST_MAX_NEW_TOKENS`, context window).
const DEFAULT_MAX_TOKENS: u32 = 256;
const DEFAULT_TEMPERATURE: f32 = 0.0;

struct Component;

#[derive(Debug, Deserialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    top_p: Option<f32>,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    stop: Option<StopField>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

/// OpenAI's `stop` is either a single string or an array of strings. Normalised
/// to a list before it is handed to the accelerator.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum StopField {
    One(String),
    Many(Vec<String>),
}

impl StopField {
    fn into_vec(self) -> Vec<String> {
        match self {
            StopField::One(value) => vec![value],
            StopField::Many(values) => values,
        }
    }
}

#[derive(Debug, Serialize)]
struct ChatCompletionResponse {
    id: &'static str,
    object: &'static str,
    created: u64,
    model: String,
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Serialize)]
struct ChatChoice {
    index: u32,
    message: ChatMessage,
    finish_reason: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelInfo {
    alias: String,
    engine: String,
    vram_required_mb: u64,
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
        match route_request(&req.method, route_path(&req.uri), &req.body) {
            Ok((status, body)) => json_response(status, body),
            Err(error) => openai_error(500, error, "server_error"),
        }
    }
}

fn route_request(method: &str, path: &str, body: &[u8]) -> Result<(u16, Vec<u8>), String> {
    // ── Registry write/list/deregister ───────────────────────────────────────
    if method.eq_ignore_ascii_case("POST") && path == ROUTE_REGISTER {
        let info: ModelInfo = serde_json::from_slice(body)
            .map_err(|e| format!("invalid model registration payload: {e}"))?;
        let value =
            serde_json::to_vec(&info).map_err(|e| format!("failed to encode model info: {e}"))?;
        models_table()
            .set(&info.alias, &value)
            .map_err(|e| format!("model registry write failed: {e}"))?;
        return Ok((201, b"model registered".to_vec()));
    }

    if method.eq_ignore_ascii_case("DELETE") && path.starts_with(ROUTE_DEREGISTER_PREFIX) {
        let alias = path.trim_start_matches(ROUTE_DEREGISTER_PREFIX);
        models_table()
            .delete(alias)
            .map_err(|e| format!("model registry delete failed: {e}"))?;
        return Ok((204, Vec::new()));
    }

    // ── OpenAI-compatible surface ─────────────────────────────────────────────
    if method.eq_ignore_ascii_case("GET") && path == ROUTE_MODELS {
        return handle_list_models();
    }

    if method.eq_ignore_ascii_case("POST") && path == ROUTE_CHAT_COMPLETIONS {
        return handle_chat_completions(body);
    }

    Ok(openai_error_payload(
        404,
        format!("route `{method} {path}` not found"),
        "invalid_request_error",
    ))
}

/// Run `/v1/chat/completions` against the host CPU accelerator: load the named
/// model (lazily materialised by the host from the broker upload directory on
/// first use), hand the structured conversation and sampling parameters to the
/// host (which renders the model's chat template and samples), and reshape the
/// output into an OpenAI-compatible response. A model the host cannot load
/// surfaces as 404.
fn handle_chat_completions(body: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let request: ChatCompletionRequest = serde_json::from_slice(body)
        .map_err(|e| format!("invalid chat completion request: {e}"))?;
    if request.model.trim().is_empty() {
        return Err("chat completion request must name a model".to_owned());
    }
    if request.messages.is_empty() {
        return Err("chat completion request must include at least one message".to_owned());
    }

    let model_id = match bindings::tachyon::accelerator::cpu::load_model(&request.model) {
        Ok(model_id) => model_id,
        Err(error) => {
            return Ok(openai_error_payload(
                404,
                format!("model `{}` is unavailable: {error}", request.model),
                "model_not_found",
            ))
        }
    };

    let generation = build_generation_request(&request)?;
    let output = bindings::tachyon::accelerator::cpu::compute(model_id, &generation)
        .map_err(|e| format!("inference failed for model `{}`: {e}", request.model))?;

    let response = ChatCompletionResponse {
        id: "chatcmpl-tachyon",
        object: "chat.completion",
        created: 0,
        model: request.model,
        choices: vec![ChatChoice {
            index: 0,
            message: ChatMessage {
                role: "assistant".to_owned(),
                content: output,
            },
            finish_reason: "stop",
        }],
    };
    serde_json::to_vec(&response)
        .map(|body| (200, body))
        .map_err(|e| format!("failed to encode chat completion response: {e}"))
}

/// Encode the host generation request: the structured chat turns (the host
/// renders the model's own chat template) plus the resolved sampling
/// parameters. Defaults are applied here so the host always receives concrete
/// values; the host still clamps them to its hard caps.
fn build_generation_request(request: &ChatCompletionRequest) -> Result<String, String> {
    let mut payload = serde_json::json!({
        "messages": request.messages,
        "max_new_tokens": request.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        "temperature": request.temperature.unwrap_or(DEFAULT_TEMPERATURE),
    });
    let object = payload
        .as_object_mut()
        .ok_or("generation payload must be a JSON object")?;
    if let Some(top_p) = request.top_p {
        object.insert("top_p".to_owned(), serde_json::json!(top_p));
    }
    if let Some(seed) = request.seed {
        object.insert("seed".to_owned(), serde_json::json!(seed));
    }
    if let Some(stop) = request.stop.clone() {
        object.insert("stop".to_owned(), serde_json::json!(stop.into_vec()));
    }
    serde_json::to_string(&payload).map_err(|e| format!("failed to encode generation request: {e}"))
}

/// Read the registry fresh from kv-partition and reshape into the OpenAI
/// `/v1/models` response. No caching — a newly registered model is visible on
/// the next call from any instance.
fn handle_list_models() -> Result<(u16, Vec<u8>), String> {
    let data = list_models()?
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

fn list_models() -> Result<Vec<ModelInfo>, String> {
    let rows = models_table()
        .get_range("", "\u{10ffff}", 10_000, 0)
        .map_err(|e| format!("model registry scan failed: {e}"))?;
    Ok(rows
        .into_iter()
        .filter_map(|(_, v)| serde_json::from_slice::<ModelInfo>(&v).ok())
        .collect())
}

fn models_table() -> bindings::tachyon::mesh::kv_partition::Table {
    bindings::tachyon::mesh::kv_partition::Table::new(MODELS_TABLE)
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

fn openai_error_payload(status: u16, message: String, kind: &'static str) -> (u16, Vec<u8>) {
    let body = serde_json::to_vec(&OpenAiError {
        error: OpenAiErrorBody { message, kind },
    })
    .unwrap_or_default();
    (status, body)
}

fn openai_error(
    status: u16,
    message: String,
    kind: &'static str,
) -> bindings::exports::tachyon::mesh::handler::Response {
    let (status, body) = openai_error_payload(status, message, kind);
    json_response(status, body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_rejects_malformed_payload() {
        let result = route_request("POST", ROUTE_REGISTER, b"not json");
        assert!(result.is_err());
    }

    #[test]
    fn chat_completions_rejects_a_malformed_request() {
        // A body with neither `model` nor `messages` is rejected during request
        // validation, before any host accelerator import is invoked.
        let result = route_request("POST", ROUTE_CHAT_COMPLETIONS, b"{}");
        assert!(result.is_err());
    }

    #[test]
    fn build_generation_request_carries_messages_and_defaults() {
        // No sampling params set: messages are forwarded structurally (the host
        // owns chat templating) and the defaults are made concrete.
        let request = ChatCompletionRequest {
            model: "m".to_owned(),
            messages: vec![
                ChatMessage {
                    role: "system".to_owned(),
                    content: "be terse".to_owned(),
                },
                ChatMessage {
                    role: "user".to_owned(),
                    content: "hello".to_owned(),
                },
            ],
            max_tokens: None,
            temperature: None,
            top_p: None,
            seed: None,
            stop: None,
        };
        let payload: serde_json::Value =
            serde_json::from_str(&build_generation_request(&request).expect("encode"))
                .expect("valid json");
        assert_eq!(payload["messages"][0]["role"], "system");
        assert_eq!(payload["messages"][1]["content"], "hello");
        assert_eq!(payload["max_new_tokens"], DEFAULT_MAX_TOKENS);
        // Optional params are omitted when unset, so the host applies its own.
        assert!(payload.get("top_p").is_none());
        assert!(payload.get("seed").is_none());
        assert!(payload.get("stop").is_none());
    }

    #[test]
    fn build_generation_request_forwards_sampling_params_and_normalizes_stop() {
        let request = ChatCompletionRequest {
            model: "m".to_owned(),
            messages: vec![ChatMessage {
                role: "user".to_owned(),
                content: "hi".to_owned(),
            }],
            max_tokens: Some(32),
            temperature: Some(0.7),
            top_p: Some(0.9),
            seed: Some(7),
            stop: Some(StopField::One("\n\n".to_owned())),
        };
        let payload: serde_json::Value =
            serde_json::from_str(&build_generation_request(&request).expect("encode"))
                .expect("valid json");
        assert_eq!(payload["max_new_tokens"], 32);
        assert_eq!(payload["seed"], 7);
        // A scalar `stop` is normalised to a single-element array for the host.
        assert_eq!(payload["stop"], serde_json::json!(["\n\n"]));
    }

    #[test]
    fn unknown_route_returns_404_shape() {
        let (status, body) =
            route_request("GET", "/unknown", b"").expect("unknown route should return a response");
        assert_eq!(status, 404);
        let parsed: serde_json::Value =
            serde_json::from_slice(&body).expect("serialized error should parse");
        assert!(parsed["error"]["message"].is_string());
    }

    #[test]
    fn deregister_prefix_strips_alias() {
        let path = "/internal/guest-openai/deregister/my-model";
        assert_eq!(path.trim_start_matches(ROUTE_DEREGISTER_PREFIX), "my-model");
    }
}
