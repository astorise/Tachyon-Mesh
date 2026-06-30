//! `guest-openai` — a user-space FaaS that provides Tachyon's OpenAI-compatible
//! HTTP surface AND owns the AI model registry.
//!
//! It is the merger of the former `system-faas-openai-adapter` and
//! `system-faas-ai-list-model` system FaaS into a single user-role example.
//! Its `openai-faas-guest` world imports `kv-partition` (the shared
//! `ai-models-registry` table is read and written directly — no separate
//! registry FaaS and no outbound mesh hop) and `tachyon:accelerator/cpu`, so
//! `/ai/v1/chat/completions` runs real inference on the host CPU accelerator.
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
const ROUTE_MODELS: &str = "/ai/v1/models";
const ROUTE_CHAT_COMPLETIONS: &str = "/ai/v1/chat/completions";
const ROUTE_EMBEDDINGS: &str = "/ai/v1/embeddings";

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
    tools: Vec<serde_json::Value>,
    #[serde(default)]
    tool_choice: Option<serde_json::Value>,
    #[serde(default)]
    tool_call_parser: Option<ToolCallParser>,
    #[serde(default)]
    extra_body: Option<ExtraBody>,
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
    #[serde(default)]
    stream: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingsRequest {
    model: String,
    input: EmbeddingsInput,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
enum EmbeddingsInput {
    One(String),
    Many(Vec<String>),
}

impl EmbeddingsInput {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(value) => vec![value],
            Self::Many(values) => values,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<ToolCall>,
}

#[cfg(test)]
impl ChatMessage {
    fn text(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: Some(content.into()),
            tool_calls: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ToolCallParser {
    Json,
    Qwen,
    QwenCoder,
    Mistral,
}

impl ToolCallParser {
    fn as_str(self) -> &'static str {
        match self {
            ToolCallParser::Json => "json",
            ToolCallParser::Qwen => "qwen",
            ToolCallParser::QwenCoder => "qwen_coder",
            ToolCallParser::Mistral => "mistral",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ExtraBody {
    #[serde(default)]
    tool_call_parser: Option<ToolCallParser>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: ToolCallFunction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Debug)]
struct ParsedAssistantOutput {
    content: String,
    tool_calls: Vec<ToolCall>,
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

/// Sent for each SSE chunk when `stream: true`.
#[derive(Debug, Serialize)]
struct ChatCompletionChunk {
    id: &'static str,
    object: &'static str,
    created: u64,
    model: String,
    choices: Vec<ChunkChoice>,
}

#[derive(Debug, Serialize)]
struct ChunkChoice {
    index: u32,
    delta: ChunkDelta,
    finish_reason: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct ChunkDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
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
struct EmbeddingsResponse {
    object: &'static str,
    data: Vec<EmbeddingData>,
    model: String,
}

#[derive(Debug, Serialize)]
struct EmbeddingData {
    object: &'static str,
    embedding: Vec<f32>,
    index: usize,
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

    if method.eq_ignore_ascii_case("POST") && path == ROUTE_EMBEDDINGS {
        return handle_embeddings(body);
    }

    Ok(openai_error_payload(
        404,
        format!("route `{method} {path}` not found"),
        "invalid_request_error",
    ))
}

fn handle_embeddings(body: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let request: EmbeddingsRequest =
        serde_json::from_slice(body).map_err(|e| format!("invalid embeddings request: {e}"))?;
    if request.model.trim().is_empty() {
        return Err("embeddings request must name a model".to_owned());
    }
    let inputs = request.input.into_vec();
    if inputs.is_empty() || inputs.iter().any(|input| input.trim().is_empty()) {
        return Err("embeddings request must include at least one non-empty input".to_owned());
    }

    let models = list_models()?;
    let alias = match resolve_model_alias(&request.model, &models) {
        Some(alias) => alias,
        None if !request.model.contains('/') => request.model.as_str(),
        None => {
            return Ok(openai_error_payload(
                404,
                format!("model `{}` is unavailable", request.model),
                "model_not_found",
            ));
        }
    };

    let model_id = match bindings::tachyon::accelerator::cpu::load_model(alias) {
        Ok(model_id) => model_id,
        Err(error) => {
            return Ok(openai_error_payload(
                404,
                format!("model `{}` is unavailable: {error}", request.model),
                "model_not_found",
            ))
        }
    };

    let mut data = Vec::with_capacity(inputs.len());
    for (index, input) in inputs.into_iter().enumerate() {
        let embedding = bindings::tachyon::accelerator::cpu::embed(model_id, &input)
            .map_err(|e| format!("embedding failed for model `{}`: {e}", request.model))?;
        data.push(EmbeddingData {
            object: "embedding",
            embedding,
            index,
        });
    }

    let response = EmbeddingsResponse {
        object: "list",
        data,
        model: request.model,
    };
    serde_json::to_vec(&response)
        .map(|body| (200, body))
        .map_err(|e| format!("failed to encode embeddings response: {e}"))
}

/// Run `/ai/v1/chat/completions` against the host CPU accelerator.
/// Routes to the streaming SSE path when `stream: true`, buffered otherwise.
fn handle_chat_completions(body: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let request: ChatCompletionRequest = serde_json::from_slice(body)
        .map_err(|e| format!("invalid chat completion request: {e}"))?;
    if request.model.trim().is_empty() {
        return Err("chat completion request must name a model".to_owned());
    }
    if request.messages.is_empty() {
        return Err("chat completion request must include at least one message".to_owned());
    }

    let models = list_models()?;
    let alias = match resolve_model_alias(&request.model, &models) {
        Some(alias) => alias,
        None if !request.model.contains('/') => request.model.as_str(),
        None => {
            return Ok(openai_error_payload(
                404,
                format!("model `{}` is unavailable", request.model),
                "model_not_found",
            ));
        }
    };

    let model_id = match bindings::tachyon::accelerator::cpu::load_model(alias) {
        Ok(model_id) => model_id,
        Err(error) => {
            return Ok(openai_error_payload(
                404,
                format!("model `{}` is unavailable: {error}", request.model),
                "model_not_found",
            ))
        }
    };

    if request.stream == Some(true) {
        handle_chat_completions_streaming(request, model_id)
    } else {
        handle_chat_completions_buffered(request, model_id)
    }
}

fn resolve_model_alias<'a>(requested: &str, models: &'a [ModelInfo]) -> Option<&'a str> {
    models
        .iter()
        .find(|model| {
            model.alias == requested || format!("{}/{}", model.engine, model.alias) == requested
        })
        .map(|model| model.alias.as_str())
}

fn handle_chat_completions_buffered(
    request: ChatCompletionRequest,
    model_id: u32,
) -> Result<(u16, Vec<u8>), String> {
    let generation = build_generation_request(&request)?;
    let output = bindings::tachyon::accelerator::cpu::compute(model_id, &generation)
        .map_err(|e| format!("inference failed for model `{}`: {e}", request.model))?;
    let parsed = parse_assistant_output(&request, &output);
    let finish_reason = if parsed.tool_calls.is_empty() {
        "stop"
    } else {
        "tool_calls"
    };

    let response = ChatCompletionResponse {
        id: "chatcmpl-tachyon",
        object: "chat.completion",
        created: 0,
        model: request.model,
        choices: vec![ChatChoice {
            index: 0,
            message: ChatMessage {
                role: "assistant".to_owned(),
                content: if parsed.content.is_empty() {
                    None
                } else {
                    Some(parsed.content)
                },
                tool_calls: parsed.tool_calls,
            },
            finish_reason,
        }],
    };
    serde_json::to_vec(&response)
        .map(|body| (200, body))
        .map_err(|e| format!("failed to encode chat completion response: {e}"))
}

/// Stream `/ai/v1/chat/completions` as Server-Sent Events. Each decoded token
/// fragment becomes a `chat.completion.chunk` frame; the stream is terminated
/// by `data: [DONE]`. The host `streaming-response` resource is used to flush
/// status + headers first, then each SSE frame as it is produced.
fn handle_chat_completions_streaming(
    request: ChatCompletionRequest,
    model_id: u32,
) -> Result<(u16, Vec<u8>), String> {
    let writer = bindings::tachyon::mesh::response_body::get_streaming_response()
        .map_err(|e| format!("streaming not available for this request: {e}"))?;

    writer
        .begin(
            200,
            &[
                ("content-type".to_string(), "text/event-stream".to_string()),
                ("cache-control".to_string(), "no-cache".to_string()),
                ("x-accel-buffering".to_string(), "no".to_string()),
            ],
        )
        .map_err(|e| format!("failed to begin streaming response: {e}"))?;

    // First chunk carries the role.
    let first_chunk = ChatCompletionChunk {
        id: "chatcmpl-tachyon",
        object: "chat.completion.chunk",
        created: 0,
        model: request.model.clone(),
        choices: vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta {
                role: Some("assistant"),
                content: None,
            },
            finish_reason: None,
        }],
    };
    write_sse_chunk(&writer, &first_chunk)?;

    let generation = build_generation_request(&request)?;
    let token_stream = bindings::tachyon::accelerator::cpu::compute_stream(model_id, &generation)
        .map_err(|e| {
        format!(
            "failed to start streaming inference for `{}`: {e}",
            request.model
        )
    })?;

    loop {
        match token_stream.next() {
            Ok(Some(fragment)) => {
                let chunk = ChatCompletionChunk {
                    id: "chatcmpl-tachyon",
                    object: "chat.completion.chunk",
                    created: 0,
                    model: request.model.clone(),
                    choices: vec![ChunkChoice {
                        index: 0,
                        delta: ChunkDelta {
                            role: None,
                            content: Some(fragment),
                        },
                        finish_reason: None,
                    }],
                };
                write_sse_chunk(&writer, &chunk)?;
            }
            Ok(None) => break,
            Err(e) => return Err(format!("streaming inference error: {e}")),
        }
    }

    // Final chunk signals stop.
    let stop_chunk = ChatCompletionChunk {
        id: "chatcmpl-tachyon",
        object: "chat.completion.chunk",
        created: 0,
        model: request.model,
        choices: vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta {
                role: None,
                content: None,
            },
            finish_reason: Some("stop"),
        }],
    };
    write_sse_chunk(&writer, &stop_chunk)?;
    writer
        .write(b"data: [DONE]\n\n")
        .map_err(|e| format!("failed to write [DONE] frame: {e}"))?;

    // Return a dummy buffered response; the real response was sent via the
    // streaming writer. The host ignores this body when streaming was used.
    Ok((200, Vec::new()))
}

fn write_sse_chunk<T: serde::Serialize>(
    writer: &bindings::tachyon::mesh::response_body::StreamingResponse,
    chunk: &T,
) -> Result<(), String> {
    let json =
        serde_json::to_string(chunk).map_err(|e| format!("failed to encode SSE chunk: {e}"))?;
    let frame = format!("data: {json}\n\n");
    writer
        .write(frame.as_bytes())
        .map_err(|e| format!("failed to write SSE frame: {e}"))
}

fn parse_assistant_output(request: &ChatCompletionRequest, output: &str) -> ParsedAssistantOutput {
    let Some(parser) = request.resolved_tool_call_parser() else {
        return ParsedAssistantOutput {
            content: output.to_owned(),
            tool_calls: Vec::new(),
        };
    };

    match parser {
        ToolCallParser::Json => parse_json_tool_calls(output),
        ToolCallParser::Qwen | ToolCallParser::QwenCoder => parse_tagged_tool_calls(
            output,
            &[
                ("<tool_call>", "</tool_call>"),
                ("<tool_calls>", "</tool_calls>"),
            ],
        ),
        ToolCallParser::Mistral => parse_mistral_tool_calls(output),
    }
    .unwrap_or_else(|| ParsedAssistantOutput {
        content: output.to_owned(),
        tool_calls: Vec::new(),
    })
}

impl ChatCompletionRequest {
    fn resolved_tool_call_parser(&self) -> Option<ToolCallParser> {
        self.tool_call_parser
            .or_else(|| {
                self.extra_body
                    .as_ref()
                    .and_then(|body| body.tool_call_parser)
            })
            .or_else(|| parser_from_model(&self.model))
            .filter(|_| !self.tools.is_empty() || self.tool_choice.is_some())
    }
}

fn parser_from_model(model: &str) -> Option<ToolCallParser> {
    let normalized = model.to_ascii_lowercase();
    if normalized.contains("qwen") && normalized.contains("coder") {
        Some(ToolCallParser::QwenCoder)
    } else if normalized.contains("qwen") {
        Some(ToolCallParser::Qwen)
    } else if normalized.contains("mistral") {
        Some(ToolCallParser::Mistral)
    } else {
        None
    }
}

fn parse_json_tool_calls(output: &str) -> Option<ParsedAssistantOutput> {
    let trimmed = strip_markdown_json_fence(output.trim());
    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    tool_calls_from_value(&value, 0).map(|tool_calls| ParsedAssistantOutput {
        content: value
            .get("content")
            .and_then(|content| content.as_str())
            .unwrap_or_default()
            .to_owned(),
        tool_calls,
    })
}

fn parse_tagged_tool_calls(
    output: &str,
    tag_pairs: &[(&str, &str)],
) -> Option<ParsedAssistantOutput> {
    let mut tool_calls = Vec::new();
    let mut content = output.to_owned();

    for (start_tag, end_tag) in tag_pairs {
        while let Some(start) = content.find(start_tag) {
            let after_start = start + start_tag.len();
            let Some(relative_end) = content[after_start..].find(end_tag) else {
                break;
            };
            let end = after_start + relative_end;
            let payload = content[after_start..end].trim();
            if let Some(mut parsed) = parse_tool_call_payload(payload, tool_calls.len()) {
                tool_calls.append(&mut parsed);
            }
            content.replace_range(start..end + end_tag.len(), "");
        }
    }

    if tool_calls.is_empty() {
        None
    } else {
        Some(ParsedAssistantOutput {
            content: content.trim().to_owned(),
            tool_calls,
        })
    }
}

fn parse_mistral_tool_calls(output: &str) -> Option<ParsedAssistantOutput> {
    let marker = "[TOOL_CALLS]";
    let start = output.find(marker)?;
    let content = output[..start].trim().to_owned();
    let payload = output[start + marker.len()..].trim();
    parse_tool_call_payload(payload, 0).map(|tool_calls| ParsedAssistantOutput {
        content,
        tool_calls,
    })
}

fn parse_tool_call_payload(payload: &str, start_index: usize) -> Option<Vec<ToolCall>> {
    let value: serde_json::Value = serde_json::from_str(strip_markdown_json_fence(payload)).ok()?;
    tool_calls_from_value(&value, start_index)
}

fn tool_calls_from_value(value: &serde_json::Value, start_index: usize) -> Option<Vec<ToolCall>> {
    let items = if let Some(items) = value.get("tool_calls").and_then(|v| v.as_array()) {
        items
    } else if let Some(items) = value.as_array() {
        items
    } else {
        return tool_call_from_value(value, start_index).map(|tool_call| vec![tool_call]);
    };

    let tool_calls: Vec<ToolCall> = items
        .iter()
        .enumerate()
        .filter_map(|(index, item)| tool_call_from_value(item, start_index + index))
        .collect();
    (!tool_calls.is_empty()).then_some(tool_calls)
}

fn tool_call_from_value(value: &serde_json::Value, index: usize) -> Option<ToolCall> {
    let function = value.get("function").unwrap_or(value);
    let name = function
        .get("name")
        .or_else(|| value.get("name"))
        .and_then(|v| v.as_str())?
        .trim();
    if name.is_empty() {
        return None;
    }

    let arguments = function
        .get("arguments")
        .or_else(|| value.get("arguments"))
        .map(canonical_arguments)
        .unwrap_or_else(|| "{}".to_owned());
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|id| !id.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("call_tachyon_{index}"));

    Some(ToolCall {
        id,
        kind: "function".to_owned(),
        function: ToolCallFunction {
            name: name.to_owned(),
            arguments,
        },
    })
}

fn canonical_arguments(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "{}".to_owned()),
    }
}

fn strip_markdown_json_fence(value: &str) -> &str {
    let trimmed = value.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let rest = rest.strip_prefix("json").unwrap_or(rest).trim_start();
    rest.strip_suffix("```").map(str::trim).unwrap_or(trimmed)
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
    if !request.tools.is_empty() {
        object.insert("tools".to_owned(), serde_json::json!(request.tools));
    }
    if let Some(tool_choice) = &request.tool_choice {
        object.insert("tool_choice".to_owned(), tool_choice.clone());
    }
    if let Some(parser) = request.resolved_tool_call_parser() {
        object.insert(
            "tool_call_parser".to_owned(),
            serde_json::json!(parser.as_str()),
        );
    }
    serde_json::to_string(&payload).map_err(|e| format!("failed to encode generation request: {e}"))
}

/// Read the registry fresh from kv-partition and reshape into the OpenAI
/// `/ai/v1/models` response. No caching — a newly registered model is visible on
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
    fn resolves_openai_model_id_to_registry_alias() {
        let models = vec![ModelInfo {
            alias: "nvidia--Qwen3.6-35B-A3B-NVFP4".to_owned(),
            engine: "safetensors".to_owned(),
            vram_required_mb: 0,
            status: "available".to_owned(),
        }];

        assert_eq!(
            resolve_model_alias("safetensors/nvidia--Qwen3.6-35B-A3B-NVFP4", &models),
            Some("nvidia--Qwen3.6-35B-A3B-NVFP4")
        );
        assert_eq!(
            resolve_model_alias("nvidia--Qwen3.6-35B-A3B-NVFP4", &models),
            Some("nvidia--Qwen3.6-35B-A3B-NVFP4")
        );
        assert_eq!(resolve_model_alias("unknown", &models), None);
    }

    #[test]
    fn build_generation_request_carries_messages_and_defaults() {
        // No sampling params set: messages are forwarded structurally (the host
        // owns chat templating) and the defaults are made concrete.
        let request = ChatCompletionRequest {
            model: "m".to_owned(),
            messages: vec![
                ChatMessage::text("system", "be terse"),
                ChatMessage::text("user", "hello"),
            ],
            tools: Vec::new(),
            tool_choice: None,
            tool_call_parser: None,
            extra_body: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            seed: None,
            stop: None,
            stream: None,
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
            messages: vec![ChatMessage::text("user", "hi")],
            tools: Vec::new(),
            tool_choice: None,
            tool_call_parser: None,
            extra_body: None,
            max_tokens: Some(32),
            temperature: Some(0.7),
            top_p: Some(0.9),
            seed: Some(7),
            stop: Some(StopField::One("\n\n".to_owned())),
            stream: None,
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
    fn build_generation_request_forwards_tools_and_parser() {
        let request = ChatCompletionRequest {
            model: "qwen-coder".to_owned(),
            messages: vec![ChatMessage::text("user", "weather?")],
            tools: vec![serde_json::json!({
                "type": "function",
                "function": { "name": "get_weather" }
            })],
            tool_choice: Some(serde_json::json!("auto")),
            tool_call_parser: None,
            extra_body: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            seed: None,
            stop: None,
            stream: None,
        };
        let payload: serde_json::Value =
            serde_json::from_str(&build_generation_request(&request).expect("encode"))
                .expect("valid json");

        assert_eq!(payload["tools"][0]["function"]["name"], "get_weather");
        assert_eq!(payload["tool_choice"], "auto");
        assert_eq!(payload["tool_call_parser"], "qwen_coder");
    }

    #[test]
    fn json_parser_extracts_openai_tool_calls() {
        let output = r#"{"tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":{"query":"tachyon"}}}]}"#;
        let parsed = parse_json_tool_calls(output).expect("tool call should parse");

        assert_eq!(parsed.content, "");
        assert_eq!(parsed.tool_calls[0].id, "call_1");
        assert_eq!(parsed.tool_calls[0].function.name, "lookup");
        assert_eq!(
            parsed.tool_calls[0].function.arguments,
            r#"{"query":"tachyon"}"#
        );
    }

    #[test]
    fn qwen_parser_extracts_tagged_tool_call_and_preserves_text() {
        let output =
            "Let me check.\n<tool_call>{\"name\":\"search\",\"arguments\":{\"q\":\"mesh\"}}</tool_call>";
        let parsed = parse_tagged_tool_calls(output, &[("<tool_call>", "</tool_call>")])
            .expect("tagged tool call should parse");

        assert_eq!(parsed.content, "Let me check.");
        assert_eq!(parsed.tool_calls[0].function.name, "search");
        assert_eq!(parsed.tool_calls[0].id, "call_tachyon_0");
    }

    #[test]
    fn mistral_parser_extracts_tool_calls_marker() {
        let output =
            "Checking\n[TOOL_CALLS] [{\"name\":\"fetch\",\"arguments\":\"{\\\"id\\\":7}\"}]";
        let parsed = parse_mistral_tool_calls(output).expect("mistral tool calls should parse");

        assert_eq!(parsed.content, "Checking");
        assert_eq!(parsed.tool_calls[0].function.name, "fetch");
        assert_eq!(parsed.tool_calls[0].function.arguments, "{\"id\":7}");
    }

    #[test]
    fn parser_is_inactive_without_tools_or_tool_choice() {
        let request = ChatCompletionRequest {
            model: "qwen".to_owned(),
            messages: vec![ChatMessage::text("user", "hi")],
            tools: Vec::new(),
            tool_choice: None,
            tool_call_parser: None,
            extra_body: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            seed: None,
            stop: None,
            stream: None,
        };
        let parsed = parse_assistant_output(&request, r#"{"name":"search","arguments":{}}"#);

        assert!(parsed.tool_calls.is_empty());
        assert_eq!(parsed.content, r#"{"name":"search","arguments":{}}"#);
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
    fn former_v1_routes_are_not_exposed() {
        for (method, path) in [
            ("GET", "/v1/models"),
            ("POST", "/v1/chat/completions"),
            ("POST", "/v1/embeddings"),
        ] {
            let (status, _) = route_request(method, path, b"{}")
                .expect("obsolete route should return a response");
            assert_eq!(status, 404, "{method} {path} must remain absent");
        }
    }

    #[test]
    fn embeddings_input_accepts_single_string_and_array() {
        let one: EmbeddingsRequest =
            serde_json::from_slice(br#"{"model":"m","input":"hello"}"#).expect("valid request");
        let many: EmbeddingsRequest =
            serde_json::from_slice(br#"{"model":"m","input":["hello","world"]}"#)
                .expect("valid request");

        assert_eq!(one.input.into_vec(), vec!["hello".to_owned()]);
        assert_eq!(
            many.input.into_vec(),
            vec!["hello".to_owned(), "world".to_owned()]
        );
    }

    #[test]
    fn deregister_prefix_strips_alias() {
        let path = "/internal/guest-openai/deregister/my-model";
        assert_eq!(path.trim_start_matches(ROUTE_DEREGISTER_PREFIX), "my-model");
    }
}
