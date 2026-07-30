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

/// Sampling default when the request omits it, chosen so a bare request is
/// reproducible. `max_tokens` deliberately has no guest-side default: omission
/// is forwarded so the host applies the budget its backend advertises.
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
    /// OpenAI gates usage reporting on a stream behind this, because the extra
    /// final chunk breaks naive clients that assume every chunk has a choice.
    #[serde(default)]
    stream_options: Option<StreamOptions>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct StreamOptions {
    #[serde(default)]
    include_usage: bool,
}

/// OpenAI's `usage` object. Omitted entirely when the backend cannot count
/// tokens — publishing zeros would be indistinguishable from a real empty
/// generation.
#[derive(Debug, Clone, Copy, Serialize)]
struct Usage {
    prompt_tokens: u32,
    completion_tokens: u32,
    total_tokens: u32,
}

impl Usage {
    fn from_host(reported: bindings::tachyon::accelerator::cpu::TokenUsage) -> Self {
        Self {
            prompt_tokens: reported.prompt_tokens,
            completion_tokens: reported.completion_tokens,
            total_tokens: reported
                .prompt_tokens
                .saturating_add(reported.completion_tokens),
        }
    }
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
    /// Set on a `role: "tool"` turn to associate the result with the call that
    /// asked for it. Dropping it here would make the host's "messages are
    /// forwarded verbatim" contract untrue for the one field an agentic loop
    /// cannot do without: the upstream would receive a tool result tied to
    /// nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    /// Legacy function-call name on a tool turn; some upstreams still key on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[cfg(test)]
impl ChatMessage {
    fn text(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
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
    /// Parse a dialect name, returning `None` for anything unrecognized.
    ///
    /// Deliberately not `Deserialize`: the registry row this reads from is
    /// written by the host, and an unknown value there must not fail the row's
    /// deserialization — that would drop the model out of `GET /ai/v1/models`
    /// entirely over a field that is only an optimization.
    fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "json" => Some(Self::Json),
            "qwen" => Some(Self::Qwen),
            "qwen_coder" => Some(Self::QwenCoder),
            "mistral" => Some(Self::Mistral),
            _ => None,
        }
    }

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
    id: String,
    object: &'static str,
    created: u64,
    model: String,
    choices: Vec<ChatChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<Usage>,
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
    id: String,
    object: &'static str,
    created: u64,
    model: String,
    choices: Vec<ChunkChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<Usage>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<StreamToolCall>>,
}

impl ChunkDelta {
    fn role(role: &'static str) -> Self {
        Self {
            role: Some(role),
            content: None,
            tool_calls: None,
        }
    }

    fn content(content: String) -> Self {
        Self {
            role: None,
            content: Some(content),
            tool_calls: None,
        }
    }

    fn tool_calls(tool_calls: Vec<StreamToolCall>) -> Self {
        Self {
            role: None,
            content: None,
            tool_calls: Some(tool_calls),
        }
    }

    fn empty() -> Self {
        Self {
            role: None,
            content: None,
            tool_calls: None,
        }
    }
}

/// A tool call inside a streaming delta. Same shape as the buffered [`ToolCall`]
/// plus the `index` that identifies which call a delta belongs to — required by
/// the OpenAI streaming format even when, as here, each call is emitted whole.
#[derive(Debug, Serialize)]
struct StreamToolCall {
    index: u32,
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: ToolCallFunction,
}

impl StreamToolCall {
    fn from_tool_call(index: u32, call: ToolCall) -> Self {
        Self {
            index,
            id: call.id,
            kind: call.kind,
            function: call.function,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelInfo {
    alias: String,
    engine: String,
    vram_required_mb: u64,
    status: String,
    /// Tool-call dialect the host resolved for this checkpoint, from its
    /// `.tachyon-model.json` sidecar or its chat template. Held as a string
    /// rather than a `ToolCallParser` so an unrecognized value cannot fail the
    /// row and take the model out of the listing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tool_call_parser: Option<String>,
    /// Who owns this row. `"config"` marks one the host derived from the sealed
    /// manifest; absent means an upload or a registration owns it.
    ///
    /// Carried here so a round-trip through this guest preserves it. Without
    /// the field, `serde` dropped it on deserialization and the re-serialized
    /// row came back unmarked — so a registration silently *disowned* a
    /// configured alias even when it did not mean to overwrite one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

/// Marks a registry row the host derived from the sealed manifest.
const REGISTRY_SOURCE_CONFIG: &str = "config";

impl ModelInfo {
    fn is_config_owned(&self) -> bool {
        self.source.as_deref() == Some(REGISTRY_SOURCE_CONFIG)
    }

    /// Strip any ownership marker a request body carried.
    ///
    /// Only the host writes `source: "config"`. Accepting it from a
    /// registration would let a caller lock an alias against the upload path —
    /// which enforces the same rule from the other side — with no manifest
    /// entry behind the claim.
    fn clear_ownership_marker(&mut self) {
        self.source = None;
    }
}

/// Whether the manifest owns this alias's row.
///
/// A missing or unparseable row is *not* config-owned: the guard exists to
/// protect a marker that is present, and treating an unreadable row as owned
/// would make a corrupt entry permanently unfixable through this route.
fn alias_is_config_owned(alias: &str) -> bool {
    models_table()
        .get(alias)
        .ok()
        .and_then(|raw| serde_json::from_slice::<ModelInfo>(&raw).ok())
        .is_some_and(|info| info.is_config_owned())
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
        let mut info: ModelInfo = serde_json::from_slice(body)
            .map_err(|e| format!("invalid model registration payload: {e}"))?;
        // A non-dynamic configured binding is loaded eagerly at boot, so a
        // request for its alias runs that backend whatever this table says.
        // Letting a registration replace the row would advertise one model
        // while another answers — for an `openai:` binding, a third-party
        // server.
        if alias_is_config_owned(&info.alias) {
            return Ok(openai_error_payload(
                409,
                format!(
                    "model alias `{}` is claimed by a configured binding in the sealed manifest; \
                     register under a free alias, or declare the binding `dynamic`",
                    info.alias
                ),
                "invalid_request_error",
            ));
        }
        info.clear_ownership_marker();
        let value =
            serde_json::to_vec(&info).map_err(|e| format!("failed to encode model info: {e}"))?;
        models_table()
            .set(&info.alias, &value)
            .map_err(|e| format!("model registry write failed: {e}"))?;
        return Ok((201, b"model registered".to_vec()));
    }

    if method.eq_ignore_ascii_case("DELETE") && path.starts_with(ROUTE_DEREGISTER_PREFIX) {
        let alias = path.trim_start_matches(ROUTE_DEREGISTER_PREFIX);
        // Same rule on the way out. Deleting a configured row does not stop the
        // runtime serving the alias — it only removes it from
        // `GET /ai/v1/models`, leaving a model that answers but cannot be
        // discovered, and which no reload short of a config change restores.
        if alias_is_config_owned(alias) {
            return Ok(openai_error_payload(
                409,
                format!(
                    "model alias `{alias}` is claimed by a configured binding in the sealed \
                     manifest; it is removed by editing the manifest, not by deregistering"
                ),
                "invalid_request_error",
            ));
        }
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
    let registered = resolve_registered_model(&request.model, &models);
    let alias = match registered {
        Some(model) => model.alias.as_str(),
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
        let embedding = match bindings::tachyon::accelerator::cpu::embed(model_id, &input) {
            Ok(embedding) => embedding,
            Err(error) => return Ok(generation_error_payload(&request.model, error)),
        };
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
    let mut request: ChatCompletionRequest = serde_json::from_slice(body)
        .map_err(|e| format!("invalid chat completion request: {e}"))?;
    if request.model.trim().is_empty() {
        return Err("chat completion request must name a model".to_owned());
    }
    if request.messages.is_empty() {
        return Err("chat completion request must include at least one message".to_owned());
    }

    let models = list_models()?;
    let registered = resolve_registered_model(&request.model, &models);
    let alias = match registered {
        Some(model) => model.alias.clone(),
        None if !request.model.contains('/') => request.model.clone(),
        None => {
            return Ok(openai_error_payload(
                404,
                format!("model `{}` is unavailable", request.model),
                "model_not_found",
            ));
        }
    };
    // Before either handler runs: every downstream use of the parser — the
    // streaming gate, the host-side generation request, and the final parse —
    // goes through `resolved_tool_call_parser`, so filling it in once here
    // covers all three.
    request.adopt_registry_parser(registered);

    let model_id = match bindings::tachyon::accelerator::cpu::load_model(&alias) {
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

/// Find the registry row a request's `model` names, by bare alias or by the
/// `{engine}/{alias}` id `GET /ai/v1/models` advertises.
fn resolve_registered_model<'a>(requested: &str, models: &'a [ModelInfo]) -> Option<&'a ModelInfo> {
    models.iter().find(|model| {
        model.alias == requested || format!("{}/{}", model.engine, model.alias) == requested
    })
}

fn handle_chat_completions_buffered(
    request: ChatCompletionRequest,
    model_id: u32,
) -> Result<(u16, Vec<u8>), String> {
    let generation = build_generation_request(&request)?;
    // `compute_detailed` rather than `compute`: same generation, but it also
    // carries the token counts. Unlike a stream, a buffered response has no
    // trailing frame to put them in, so they have to come back with the text.
    let completed =
        match bindings::tachyon::accelerator::cpu::compute_detailed(model_id, &generation) {
            Ok(completed) => completed,
            Err(error) => return Ok(generation_error_payload(&request.model, error)),
        };
    // Structured calls win outright: the backend received them as fields, so
    // re-reading the text for calls could only ever guess worse.
    let parsed = if completed.tool_calls.is_empty() {
        parse_assistant_output(&request, &completed.text)
    } else {
        ParsedAssistantOutput {
            content: completed.text.clone(),
            tool_calls: adopt_host_tool_calls(completed.tool_calls),
        }
    };
    let finish_reason = resolve_finish_reason(
        completed.finish_reason.as_deref(),
        !parsed.tool_calls.is_empty(),
    );

    let response = ChatCompletionResponse {
        // Unconditional here, unlike the stream: a buffered response has no
        // extra frame to break a client with, so OpenAI reports usage on it
        // always. Still `None` when the backend could not measure — a zero
        // `usage` claims the generation cost nothing.
        usage: completed.usage.map(Usage::from_host),
        id: completion_id(),
        object: "chat.completion",
        created: unix_seconds(),
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
                // Response-side only: an assistant turn never carries these.
                tool_call_id: None,
                name: None,
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

    // Resolved once: every chunk of one response must carry the same id and
    // timestamp, or a client reassembling the stream sees each delta as a
    // separate completion.
    let id = completion_id();
    let created = unix_seconds();

    // First chunk carries the role.
    let first_chunk = ChatCompletionChunk {
        usage: None,
        id: id.clone(),
        object: "chat.completion.chunk",
        created,
        model: request.model.clone(),
        choices: vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta::role("assistant"),
            finish_reason: None,
        }],
    };
    write_sse_chunk(&writer, &first_chunk)?;

    // Tool intent turns this into a gated stream: content flows until an opener
    // appears, then the rest is held back for the buffered parser.
    //
    // The gate is always present, defaulting to the anchored `Json` opener when
    // the request implies no parser: the host's upstream backend emits its
    // tool-call envelope as one whole-output JSON object, and without a gate
    // that envelope would stream out as prose before finalization could turn it
    // into a structured call. For a response that is not a tool call the
    // anchored gate is a passthrough — a `{` only counts at the very start.
    // Only when the request actually offers tools. The gate exists to stop a
    // tool call streaming out as prose before it can be turned into a
    // structured call, and nothing can produce one when no tools were offered
    // — while an unconditional gate trips at byte zero on any answer that
    // starts with `{`, `[` or a fence, which is what ordinary JSON and code
    // answers do. That withheld every fragment until generation finished,
    // costing the streaming route exactly the time-to-first-token it exists
    // for.
    let mut gate = request.has_tool_intent().then(|| {
        StreamingContentGate::new(
            request
                .resolved_tool_call_parser()
                .unwrap_or(ToolCallParser::Json),
        )
    });

    let generation = build_generation_request(&request)?;
    let token_stream =
        match bindings::tachyon::accelerator::cpu::compute_stream(model_id, &generation) {
            Ok(token_stream) => token_stream,
            // The headers are already on the wire, so the status can no longer
            // be changed — the failure is reported as an SSE error frame, which
            // is what an OpenAI client reads mid-stream anyway.
            Err(error) => {
                write_sse_error(&writer, &request.model, error)?;
                return Ok((200, Vec::new()));
            }
        };

    // Tool calls the host recognised as structured data, kept aside until the
    // stream ends: they are emitted as one `tool_calls` delta after the content,
    // which is where an OpenAI client expects them.
    let mut host_tool_calls = Vec::new();
    loop {
        match token_stream.next() {
            Ok(Some(bindings::tachyon::accelerator::cpu::StreamEvent::Content(fragment))) => {
                let content = match gate.as_mut() {
                    Some(gate) => gate.push(&fragment),
                    None => Some(fragment),
                };
                let Some(content) = content else {
                    continue;
                };
                let chunk = ChatCompletionChunk {
                    usage: None,
                    id: id.clone(),
                    object: "chat.completion.chunk",
                    created,
                    model: request.model.clone(),
                    choices: vec![ChunkChoice {
                        index: 0,
                        delta: ChunkDelta::content(content),
                        finish_reason: None,
                    }],
                };
                write_sse_chunk(&writer, &chunk)?;
            }
            Ok(Some(bindings::tachyon::accelerator::cpu::StreamEvent::ToolCall(call))) => {
                host_tool_calls.push(call);
            }
            Ok(None) => break,
            Err(error) => {
                write_sse_error(&writer, &request.model, error)?;
                return Ok((200, Vec::new()));
            }
        }
    }

    // Whatever the gate held back is parsed exactly like a buffered response,
    // so streamed and buffered requests recover the same tool calls.
    //
    // Structured calls win over anything parsed out of the text, for the same
    // reason as on the buffered path: the backend received them as fields.
    let mut tool_calls = adopt_host_tool_calls(host_tool_calls);
    if let Some(gate) = gate {
        let (whole, streamed) = gate.finish();
        let parsed = parse_assistant_output(&request, &whole);

        // Content the gate held back that turned out not to be part of a tool
        // call — text the model emitted *after* the call, typically.
        //
        // `parsed.content` is the whole text minus the tool-call regions and
        // trimmed at both ends, while what was streamed is a raw prefix, so the
        // two are compared trimmed. Matching by prefix rather than by byte
        // offset is what keeps this safe: if the streamed text is not a prefix
        // of the parsed content, nothing more is emitted, because duplicating
        // text in the client's transcript is worse than omitting a tail.
        // Two different reconciliations, because the two cases differ.
        //
        // When parsing found no tool calls it fell back to returning the raw
        // text unchanged, so the exact byte prefix already streamed is the
        // right cut — trimming it there would drop the gate's held-back tail on
        // a response starting with whitespace, or re-emit trailing whitespace.
        //
        // When parsing *did* extract calls, `parsed.content` is the text minus
        // those regions and trimmed at both ends, so the comparison has to be
        // trimmed too. Matching by prefix rather than by offset is what keeps
        // this safe: no match means no tail is emitted, and duplicating text in
        // the transcript is worse than omitting a trailing fragment.
        let tail = if parsed.tool_calls.is_empty() {
            parsed.content.get(streamed..).unwrap_or_default()
        } else {
            // `parsed.content` is the text minus the tool-call regions, trimmed
            // at both ends. Only its *leading* trim shifts the offset of what
            // was already streamed, so the consumed length is the streamed
            // prefix minus the whitespace the parser dropped from the front.
            // Trimming the prefix at both ends instead would hand back its own
            // trailing whitespace as new content — three newlines where the
            // buffered parser produces two.
            let already_streamed = whole.get(..streamed).unwrap_or_default();
            let consumed = already_streamed.trim_start().len();
            parsed.content.get(consumed..).unwrap_or_default()
        };
        if !tail.is_empty() {
            let chunk = ChatCompletionChunk {
                usage: None,
                id: id.clone(),
                object: "chat.completion.chunk",
                created,
                model: request.model.clone(),
                choices: vec![ChunkChoice {
                    index: 0,
                    delta: ChunkDelta::content(tail.to_owned()),
                    finish_reason: None,
                }],
            };
            write_sse_chunk(&writer, &chunk)?;
        }

        if tool_calls.is_empty() {
            tool_calls = parsed.tool_calls;
        }
    }

    // Read now rather than at the top: like `usage`, it is only known once the
    // stream has ended, which the loop above has just observed.
    let host_finish_reason = token_stream.finish_reason();
    let finish_reason =
        resolve_finish_reason(host_finish_reason.as_deref(), !tool_calls.is_empty());
    if !tool_calls.is_empty() {
        let deltas = tool_calls
            .into_iter()
            .enumerate()
            .map(|(index, call)| StreamToolCall::from_tool_call(index as u32, call))
            .collect();
        let chunk = ChatCompletionChunk {
            usage: None,
            id: id.clone(),
            object: "chat.completion.chunk",
            created,
            model: request.model.clone(),
            choices: vec![ChunkChoice {
                index: 0,
                delta: ChunkDelta::tool_calls(deltas),
                finish_reason: None,
            }],
        };
        write_sse_chunk(&writer, &chunk)?;
    }

    // Final chunk signals why generation ended.
    let stop_chunk = ChatCompletionChunk {
        id: id.clone(),
        object: "chat.completion.chunk",
        created,
        model: request.model.clone(),
        choices: vec![ChunkChoice {
            index: 0,
            delta: ChunkDelta::empty(),
            finish_reason: Some(finish_reason),
        }],
        usage: None,
    };
    write_sse_chunk(&writer, &stop_chunk)?;

    // Usage rides in its own trailing chunk with no choices, which is where
    // OpenAI puts it — and why it is gated behind `stream_options.include_usage`
    // there: a client that assumes every chunk has a `choices[0]` breaks on it.
    // Read only now, because the counts are not known until decoding ends.
    if request
        .stream_options
        .as_ref()
        .is_some_and(|options| options.include_usage)
    {
        if let Some(reported) = token_stream.usage() {
            let usage_chunk = ChatCompletionChunk {
                id,
                object: "chat.completion.chunk",
                created,
                model: request.model,
                choices: Vec::new(),
                usage: Some(Usage::from_host(reported)),
            };
            write_sse_chunk(&writer, &usage_chunk)?;
        }
    }

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

/// Openers that mark the start of a tool call for a given parser, and whether
/// they can only appear at the very beginning of the output.
///
/// `Json` is anchored: `parse_json_tool_calls` requires the *whole* output to be
/// one JSON value, so a `{` anywhere but the start cannot begin a tool call —
/// parsing would fail and the text stays content. The tagged parsers scan
/// anywhere, because those models routinely emit prose and then a call.
fn tool_call_openers(parser: ToolCallParser) -> (&'static [&'static str], bool) {
    match parser {
        // `[` matters as much as `{`: `tool_calls_from_value` accepts a bare
        // top-level array of calls, so omitting it would stream the payload as
        // content *and* emit it again as a structured call.
        ToolCallParser::Json => (&["{", "[", "```"], true),
        ToolCallParser::Qwen | ToolCallParser::QwenCoder => {
            (&["<tool_call>", "<tool_calls>"], false)
        }
        ToolCallParser::Mistral => (&["[TOOL_CALLS]"], false),
    }
}

/// Decides, as fragments arrive, how much of a streamed response can safely be
/// forwarded as assistant content.
///
/// Buffering the whole generation before parsing would be simplest, but it
/// destroys time-to-first-token for every request that merely *offers* tools —
/// which, for an agentic client, is every request. So content is streamed until
/// an opener appears; from there everything is held for the buffered parser,
/// because the tool-call region is not content and must not leak into the
/// client's transcript.
///
/// Bytes are held back near the tail so an opener split across two fragments is
/// still matched, the same trick the host's decode loop uses for stop
/// sequences.
struct StreamingContentGate {
    openers: &'static [&'static str],
    anchored: bool,
    hold: usize,
    seen: String,
    emitted: usize,
    tripped: bool,
}

impl StreamingContentGate {
    fn new(parser: ToolCallParser) -> Self {
        let (openers, anchored) = tool_call_openers(parser);
        let hold = openers
            .iter()
            .map(|opener| opener.len())
            .max()
            .unwrap_or(1)
            .saturating_sub(1);
        Self {
            openers,
            anchored,
            hold,
            seen: String::new(),
            emitted: 0,
            tripped: false,
        }
    }

    /// Absorb a fragment, returning the content safe to stream right now.
    fn push(&mut self, fragment: &str) -> Option<String> {
        self.seen.push_str(fragment);
        if self.tripped {
            return None;
        }
        if let Some(at) = self.find_opener() {
            self.tripped = true;
            // Everything before the opener is genuine content — minus the
            // whitespace that separates it from the call. The buffered parser
            // removes the call region and `trim()`s what is left, so emitting
            // the newline in `Let me check.\n<tool_call>…` would leave the
            // concatenated deltas differing from the buffered message by
            // exactly that whitespace, and the streaming contract is that the
            // two are equal.
            //
            // `emitted` still advances to the opener: the whitespace is
            // accounted for, not pending, so the caller's tail reconciliation
            // does not hand it back afterwards.
            let content = self.seen[self.emitted..at].trim_end().to_owned();
            self.emitted = at;
            return (!content.is_empty()).then_some(content);
        }
        let ceiling = floor_char_boundary(&self.seen, self.seen.len().saturating_sub(self.hold));
        // Trailing whitespace is withheld for the same reason, before we know
        // whether an opener follows it. Nothing is lost when none does: the
        // buffered parse then returns the text unchanged, and the caller emits
        // whatever it kept beyond what was streamed.
        let safe = self.seen[..ceiling].trim_end().len();
        if safe <= self.emitted {
            return None;
        }
        let content = self.seen[self.emitted..safe].to_owned();
        self.emitted = safe;
        Some(content)
    }

    fn find_opener(&self) -> Option<usize> {
        if self.anchored {
            // Anchored openers only count at the start, ignoring leading
            // whitespace the model may have emitted first.
            let trimmed = self.seen.trim_start();
            let offset = self.seen.len() - trimmed.len();
            return self
                .openers
                .iter()
                .any(|opener| trimmed.starts_with(opener))
                .then_some(offset);
        }
        self.openers
            .iter()
            .filter_map(|opener| self.seen.find(opener))
            .min()
    }

    /// The whole generation, for the buffered parser to work on.
    fn finish(self) -> (String, usize) {
        (self.seen, self.emitted)
    }
}

/// Largest index `idx` that is a char boundary of `text`. Mirrors the host's own
/// helper; `str::floor_char_boundary` is still unstable.
fn floor_char_boundary(text: &str, mut idx: usize) -> usize {
    if idx >= text.len() {
        return text.len();
    }
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Adopt the tool calls the host recognised as structured data.
///
/// These need no parsing and no dialect guess: the backend received them as
/// fields, not as text. That is the whole reason the accelerator interface has
/// a `tool-call` channel — smuggled through the text channel, a call decodes
/// only when the request happens to carry the nonstandard parser option, so a
/// standard OpenAI client offering tools would get the raw JSON back as literal
/// assistant prose.
fn adopt_host_tool_calls(
    calls: Vec<bindings::tachyon::accelerator::cpu::ToolCall>,
) -> Vec<ToolCall> {
    calls
        .into_iter()
        .enumerate()
        .map(|(index, call)| ToolCall {
            // A provider that assigned no id gets one minted here, matching the
            // parser's own scheme: the id is what a client echoes back on the
            // tool result turn, so it cannot be left empty.
            id: call.id.unwrap_or_else(|| format!("call_tachyon_{index}")),
            kind: "function".to_owned(),
            function: ToolCallFunction {
                name: call.name,
                arguments: call.arguments,
            },
        })
        .collect()
}

/// The `finish_reason` a choice reports, given what the host said and whether
/// any tool call was recovered.
///
/// `length` outranks `tool_calls`, which is the one ordering that is not
/// obvious. A model that runs out of budget *while emitting a call* returns
/// both a partial `tool_calls` entry and `length`; reporting `tool_calls` there
/// tells the client the call is ready and invites it to dispatch truncated
/// arguments — a half-written path, a half-written patch. `length` tells it the
/// truth, and a client that checks the reason before dispatching is protected.
/// `content_filter` outranks it for the same reason: the answer is not the
/// model's own.
fn resolve_finish_reason(host_reported: Option<&str>, has_tool_calls: bool) -> &'static str {
    match host_reported {
        Some("length") => "length",
        Some("content_filter") => "content_filter",
        _ if has_tool_calls => "tool_calls",
        Some("tool_calls") => "tool_calls",
        // Anything else, including an absent reason, is an ordinary completion
        // as far as the OpenAI schema is concerned.
        _ => "stop",
    }
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
    /// Adopt the dialect the host resolved for this model, unless the caller
    /// named one explicitly.
    ///
    /// This is what makes tool calling a property of the *checkpoint* rather
    /// than of its alias. Without it the only automatic source is
    /// [`parser_from_model`], which matches on the alias string — so a Qwen
    /// checkpoint registered as `local-coder` got no parser at all and emitted
    /// its `<tool_call>` blocks as literal assistant text. For an agentic
    /// client that reads as "the model declined to call a tool", which is
    /// indistinguishable from success and impossible to debug from outside.
    fn adopt_registry_parser(&mut self, model: Option<&ModelInfo>) {
        if self.tool_call_parser.is_some()
            || self
                .extra_body
                .as_ref()
                .is_some_and(|body| body.tool_call_parser.is_some())
        {
            // The caller was explicit; the registry does not get to override.
            return;
        }
        self.tool_call_parser = model
            .and_then(|model| model.tool_call_parser.as_deref())
            .and_then(ToolCallParser::from_name);
    }

    /// Whether the caller offered the model any tool to call.
    fn has_tool_intent(&self) -> bool {
        !self.tools.is_empty() || self.tool_choice.is_some()
    }

    fn resolved_tool_call_parser(&self) -> Option<ToolCallParser> {
        self.tool_call_parser
            .or_else(|| {
                self.extra_body
                    .as_ref()
                    .and_then(|body| body.tool_call_parser)
            })
            .or_else(|| parser_from_model(&self.model))
            .filter(|_| self.has_tool_intent())
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
        "temperature": request.temperature.unwrap_or(DEFAULT_TEMPERATURE),
    });
    let object = payload
        .as_object_mut()
        .ok_or("generation payload must be a JSON object")?;
    // Omission is forwarded as omission. Substituting a default here would
    // override whatever budget the *binding* advertises — an upstream binding
    // configured for long agentic completions would silently truncate at this
    // guest's number instead. The host applies its own default per backend.
    if let Some(max_tokens) = request.max_tokens {
        object.insert("max_new_tokens".to_owned(), serde_json::json!(max_tokens));
    }
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

/// Seconds since the Unix epoch, or `0` when the host denies a clock.
///
/// OpenAI clients display `created`; it was hardcoded to `0`, which reads as
/// January 1970 in every one of them.
fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// A per-response completion id.
///
/// Every response used to carry the literal `chatcmpl-tachyon`, so nothing
/// downstream — a proxy log, a client cache, a trace — could tell two
/// completions apart. The value only has to be unique, not unguessable: it
/// identifies a response, it does not authorize anything.
///
/// Built from the wall clock in nanoseconds plus a per-instance counter, so
/// two responses collide only if the host reports the same nanosecond *and*
/// the counter wrapped — and the counter alone keeps ids distinct within an
/// instance even if the clock is denied and reads zero.
fn completion_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_nanos() as u64)
        .unwrap_or(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("chatcmpl-{nanos:016x}{seq:08x}")
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

/// Turn a host generation failure into the OpenAI error the client should see.
///
/// The point of relaying the upstream's status is that the client's own
/// behaviour depends on it: a 429 must engage its backoff, and a rejected
/// request must not be retried at all. Reporting every provider failure as a
/// 500 leaves it with one response — retry blindly — which is wrong for both.
///
/// Two classes are deliberately *not* relayed. An upstream auth failure is this
/// node's misconfigured credential, not the caller's, and a 401 would send it
/// chasing its own key; and an unclassified status becomes 502, because this
/// node is the gateway and the failure is the gateway's to explain.
fn generation_error_payload(
    model: &str,
    error: bindings::tachyon::accelerator::cpu::GenerationError,
) -> (u16, Vec<u8>) {
    let message = format!("inference failed for model `{model}`: {}", error.message);
    let (status, kind) = match error.upstream_status {
        Some(429) => (429, "rate_limit_error"),
        // The provider rejected the request we forwarded, which almost always
        // reflects the caller's own parameters — an unknown model, a tool
        // schema it will not accept, a context overflow.
        Some(400 | 404 | 405 | 409 | 413 | 422) => (400, "invalid_request_error"),
        Some(status @ 502..=504) => (status, "server_error"),
        Some(_) => (502, "server_error"),
        None => (500, "server_error"),
    };
    openai_error_payload(status, message, kind)
}

/// Report a failure that arrived after the response headers were already
/// flushed. The status line is spent by then, so the only honest channel left
/// is an SSE frame carrying the same error body a buffered request would get.
fn write_sse_error(
    writer: &bindings::tachyon::mesh::response_body::StreamingResponse,
    model: &str,
    error: bindings::tachyon::accelerator::cpu::GenerationError,
) -> Result<(), String> {
    let (_status, body) = generation_error_payload(model, error);
    let frame = format!(
        "data: {}\n\n",
        String::from_utf8(body).unwrap_or_else(|_| "{}".to_owned())
    );
    writer
        .write(frame.as_bytes())
        .map_err(|e| format!("failed to write the streaming error frame: {e}"))?;
    writer
        .write(b"data: [DONE]\n\n")
        .map_err(|e| format!("failed to write [DONE] frame: {e}"))
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

    /// The registration route rewrites the whole row, so any field this struct
    /// does not model is erased by the round-trip. When `source` was missing,
    /// re-registering an alias stripped the manifest's ownership marker without
    /// anyone overwriting anything deliberately — and the upload path, which
    /// reads exactly that marker, then treated a configured alias as free.
    #[test]
    fn a_registry_row_round_trip_keeps_its_ownership_marker() {
        let stored = br#"{
            "alias": "shared",
            "engine": "openai",
            "vramRequiredMb": 0,
            "status": "available",
            "source": "config"
        }"#;

        let info: ModelInfo = serde_json::from_slice(stored).expect("row should parse");
        assert!(info.is_config_owned());

        let rewritten = serde_json::to_vec(&info).expect("row should re-encode");
        let reparsed: ModelInfo = serde_json::from_slice(&rewritten).expect("row should re-parse");
        assert!(
            reparsed.is_config_owned(),
            "a round-trip through this guest must not disown a configured alias"
        );
    }

    #[test]
    fn only_the_config_marker_claims_ownership() {
        // Absent means an upload or a registration owns the row — the common
        // case, and the one that must stay writable.
        assert!(!registry_model("free", None).is_config_owned());

        // An unrecognised value is not ownership either. Treating anything
        // non-empty as owned would let a stray field freeze an alias.
        let mut odd = registry_model("odd", None);
        odd.source = Some("upload".to_owned());
        assert!(!odd.is_config_owned());

        let mut owned = registry_model("owned", None);
        owned.source = Some(REGISTRY_SOURCE_CONFIG.to_owned());
        assert!(owned.is_config_owned());
    }

    /// A row this guest writes must never claim the manifest's marker: the
    /// upload path enforces the same rule from the other side, so a
    /// registration that could set `source: "config"` would lock an alias
    /// against uploads with no manifest entry backing it.
    #[test]
    fn a_registration_cannot_claim_manifest_ownership() {
        let mut info: ModelInfo = serde_json::from_slice(
            br#"{"alias":"claimed","engine":"gguf","vramRequiredMb":0,
                 "status":"available","source":"config"}"#,
        )
        .expect("payload should parse");
        assert!(info.is_config_owned(), "the request body did claim it");

        // What the register route does before writing.
        info.clear_ownership_marker();
        assert!(!info.is_config_owned());

        let encoded = serde_json::to_vec(&info).expect("row should encode");
        let value: serde_json::Value = serde_json::from_slice(&encoded).expect("row should parse");
        assert!(
            value.get("source").is_none(),
            "an unowned row omits the marker entirely rather than writing a null"
        );
    }

    #[test]
    fn chat_completions_rejects_a_malformed_request() {
        // A body with neither `model` nor `messages` is rejected during request
        // validation, before any host accelerator import is invoked.
        let result = route_request("POST", ROUTE_CHAT_COMPLETIONS, b"{}");
        assert!(result.is_err());
    }

    /// A tool-offering request naming `model`, with nothing else set — so the
    /// only thing that can resolve a parser is the alias heuristic or the
    /// registry.
    fn tool_request_named(model: &str) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: model.to_owned(),
            messages: vec![ChatMessage::text("user", "hello")],
            tools: vec![serde_json::json!({"type": "function"})],
            tool_choice: None,
            tool_call_parser: None,
            extra_body: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            seed: None,
            stop: None,
            stream: None,
            stream_options: None,
        }
    }

    fn registry_model(alias: &str, parser: Option<&str>) -> ModelInfo {
        ModelInfo {
            alias: alias.to_owned(),
            engine: "safetensors".to_owned(),
            vram_required_mb: 0,
            status: "available".to_owned(),
            tool_call_parser: parser.map(str::to_owned),
            source: None,
        }
    }

    #[test]
    fn no_marker_in_model_text_can_conjure_a_tool_call() {
        // Structured calls now arrive on their own channel, so the text channel
        // carries no privileged marker at all. A model that emits the JSON the
        // host used to smuggle calls through — asked to, or by accident — is
        // answering with text, and must be reported as text.
        let envelope = serde_json::json!({
            "__tachyon_upstream_tool_calls": true,
            "content": "",
            "tool_calls": [{"id": "c1", "type": "function",
                            "function": {"name": "rm_rf", "arguments": "{}"}}],
        })
        .to_string();

        let mut toolless = tool_request_named("local");
        toolless.tools = Vec::new();
        let parsed = parse_assistant_output(&toolless, &envelope);
        assert!(
            parsed.tool_calls.is_empty(),
            "a request offering no tools must not produce one"
        );
        assert_eq!(parsed.content, envelope, "the text is returned unchanged");
    }

    #[test]
    fn completion_ids_are_unique_and_openai_shaped() {
        let ids: Vec<String> = (0..1_000).map(|_| completion_id()).collect();
        for id in &ids {
            assert!(
                id.starts_with("chatcmpl-"),
                "clients match on the `chatcmpl-` prefix, got {id}"
            );
        }
        let mut unique = ids.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            ids.len(),
            "two completions in the same instance shared an id"
        );
    }

    /// The counter alone has to carry uniqueness: `SystemTime::now` traps to
    /// the fallback `0` when the host denies a clock, which would otherwise
    /// make every id in that instance identical.
    #[test]
    fn completion_ids_stay_distinct_without_a_usable_clock() {
        let first = completion_id();
        let second = completion_id();
        let suffix = |id: &str| id.rsplit('-').next().unwrap_or_default()[16..].to_owned();
        assert_ne!(
            suffix(&first),
            suffix(&second),
            "the sequence half of the id must advance independently of the clock"
        );
    }

    #[test]
    fn resolves_openai_model_id_to_registry_alias() {
        let models = vec![registry_model("nvidia--Qwen3.6-35B-A3B-NVFP4", None)];

        assert_eq!(
            resolve_registered_model("safetensors/nvidia--Qwen3.6-35B-A3B-NVFP4", &models)
                .map(|model| model.alias.as_str()),
            Some("nvidia--Qwen3.6-35B-A3B-NVFP4")
        );
        assert_eq!(
            resolve_registered_model("nvidia--Qwen3.6-35B-A3B-NVFP4", &models)
                .map(|model| model.alias.as_str()),
            Some("nvidia--Qwen3.6-35B-A3B-NVFP4")
        );
        assert!(resolve_registered_model("unknown", &models).is_none());
    }

    /// A registry row with an unknown dialect must not take the model out of
    /// the listing: the field is an optimization, the row is the model's
    /// existence.
    #[test]
    fn an_unknown_registry_parser_is_ignored_rather_than_fatal() {
        let row = serde_json::json!({
            "alias": "local-coder",
            "engine": "gguf",
            "vramRequiredMb": 0,
            "status": "available",
            "toolCallParser": "some-future-dialect",
        });
        let model: ModelInfo =
            serde_json::from_value(row).expect("an unknown dialect must not fail the row");

        let mut request = tool_request_named("local-coder");
        request.adopt_registry_parser(Some(&model));
        assert_eq!(request.resolved_tool_call_parser(), None);
    }

    /// The regression this whole seam exists for: a Qwen checkpoint whose alias
    /// says nothing about it. Before, `parser_from_model` found no "qwen" in
    /// `local-coder` and the model's `<tool_call>` blocks came back as prose.
    #[test]
    fn a_neutrally_named_model_still_gets_its_parser_from_the_registry() {
        let mut request = tool_request_named("local-coder");
        assert_eq!(
            request.resolved_tool_call_parser(),
            None,
            "the alias heuristic cannot classify this name — that is the bug"
        );

        request.adopt_registry_parser(Some(&registry_model("local-coder", Some("qwen"))));
        assert_eq!(
            request.resolved_tool_call_parser(),
            Some(ToolCallParser::Qwen)
        );
    }

    /// Precedence: an explicit request field outranks the registry, so a client
    /// that knows better than the checkpoint's own metadata keeps control.
    #[test]
    fn an_explicit_parser_outranks_the_registry() {
        let mut request = tool_request_named("local-coder");
        request.tool_call_parser = Some(ToolCallParser::Mistral);
        request.adopt_registry_parser(Some(&registry_model("local-coder", Some("qwen"))));
        assert_eq!(
            request.resolved_tool_call_parser(),
            Some(ToolCallParser::Mistral)
        );

        let mut via_extra_body = tool_request_named("local-coder");
        via_extra_body.extra_body = Some(ExtraBody {
            tool_call_parser: Some(ToolCallParser::Json),
        });
        via_extra_body.adopt_registry_parser(Some(&registry_model("local-coder", Some("qwen"))));
        assert_eq!(
            via_extra_body.resolved_tool_call_parser(),
            Some(ToolCallParser::Json)
        );
    }

    /// A request that offers no tools never needs a parser, whatever the
    /// registry says — the filter in `resolved_tool_call_parser` still applies.
    #[test]
    fn a_registry_parser_does_not_apply_to_a_toolless_request() {
        let mut request = tool_request_named("local-coder");
        request.tools = Vec::new();
        request.adopt_registry_parser(Some(&registry_model("local-coder", Some("qwen"))));
        assert_eq!(request.resolved_tool_call_parser(), None);
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
            stream_options: None,
        };
        let payload: serde_json::Value =
            serde_json::from_str(&build_generation_request(&request).expect("encode"))
                .expect("valid json");
        assert_eq!(payload["messages"][0]["role"], "system");
        assert_eq!(payload["messages"][1]["content"], "hello");
        // Omitted, not defaulted: substituting a number here would override the
        // budget the binding advertises — an upstream binding configured for
        // long completions would silently truncate at this guest's default.
        assert!(payload.get("max_new_tokens").is_none());
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
            stream_options: None,
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
            stream_options: None,
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

    /// Drive a gate with fragments, returning what it streamed as content and
    /// what it held back for the buffered parser.
    fn run_gate(parser: ToolCallParser, fragments: &[&str]) -> (String, String, usize) {
        let mut gate = StreamingContentGate::new(parser);
        let mut streamed = String::new();
        for fragment in fragments {
            if let Some(content) = gate.push(fragment) {
                streamed.push_str(&content);
            }
        }
        let (whole, emitted) = gate.finish();
        (streamed, whole, emitted)
    }

    #[test]
    fn streamed_content_equals_the_buffered_message_across_a_tool_call() {
        // The streaming contract is that concatenating the content deltas
        // yields the buffered message. The whitespace between prose and an
        // opener is where the two used to diverge: the buffered parser removes
        // the call region and trims, while the gate had already streamed the
        // newline.
        let (streamed, whole, _emitted) = run_gate(
            ToolCallParser::Qwen,
            &[
                "Let me check.",
                "\n",
                "<tool_call>",
                r#"{"name":"read_file","arguments":{}}"#,
                "</tool_call>",
            ],
        );
        let mut request = tool_request_named("local");
        request.tool_call_parser = Some(ToolCallParser::Qwen);
        let parsed = parse_assistant_output(&request, &whole);
        assert_eq!(
            parsed.tool_calls.len(),
            1,
            "the call must still be recovered"
        );
        assert_eq!(
            streamed, parsed.content,
            "streamed content must equal the buffered message, whitespace included"
        );
    }

    #[test]
    fn withheld_trailing_whitespace_is_released_when_no_call_follows() {
        // The other side of the same rule: whitespace is only *deferred*, never
        // dropped. With no call, the buffered parse returns the text unchanged
        // and the caller's reconciliation emits whatever the gate still held.
        let (streamed, whole, emitted) = run_gate(
            ToolCallParser::Qwen,
            &[
                "Here it is, at some length so the opener hold is not the binding constraint.",
                "\n\n",
            ],
        );
        assert!(
            !streamed.ends_with(char::is_whitespace),
            "trailing whitespace is withheld while an opener could still follow, got {streamed:?}"
        );
        assert_eq!(
            format!("{streamed}{}", whole.get(emitted..).unwrap_or_default()),
            whole,
            "streamed content plus the caller's tail must reconstruct the whole generation"
        );
    }

    #[test]
    fn host_reported_tool_calls_need_no_parser_selection() {
        // A standard OpenAI client offers tools and passes no parser option;
        // the model name implies none either. Structured calls arrive on their
        // own channel, so no dialect has to be guessed for them to survive —
        // which is what previously decided whether a call reached the client at
        // all or arrived as literal prose.
        let calls = adopt_host_tool_calls(vec![bindings::tachyon::accelerator::cpu::ToolCall {
            id: Some("c1".to_owned()),
            name: "read_file".to_owned(),
            arguments: r#"{"path":"a.rs"}"#.to_owned(),
        }]);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "c1");
        assert_eq!(calls[0].kind, "function");
        assert_eq!(calls[0].function.name, "read_file");
        assert_eq!(calls[0].function.arguments, r#"{"path":"a.rs"}"#);
    }

    #[test]
    fn a_host_tool_call_without_an_id_is_given_one() {
        // The id is what a client echoes back on the tool-result turn, so an
        // empty one makes the call impossible to answer.
        let calls = adopt_host_tool_calls(vec![bindings::tachyon::accelerator::cpu::ToolCall {
            id: None,
            name: "f".to_owned(),
            arguments: "{}".to_owned(),
        }]);
        assert_eq!(calls[0].id, "call_tachyon_0");
    }

    #[test]
    fn a_truncated_tool_call_reports_length_not_tool_calls() {
        // The dangerous case: the upstream ran out of budget mid-call, so the
        // arguments are incomplete. Reporting `tool_calls` tells the client the
        // call is ready to dispatch.
        assert_eq!(resolve_finish_reason(Some("length"), true), "length");
        assert_eq!(
            resolve_finish_reason(Some("content_filter"), true),
            "content_filter"
        );
        // A complete call still reports `tool_calls`, whether the host named it
        // or the parser recovered it from the text.
        assert_eq!(resolve_finish_reason(Some("stop"), true), "tool_calls");
        assert_eq!(resolve_finish_reason(None, true), "tool_calls");
        assert_eq!(
            resolve_finish_reason(Some("tool_calls"), false),
            "tool_calls"
        );
        // And an ordinary completion is `stop`, including when nothing was
        // reported at all.
        assert_eq!(resolve_finish_reason(None, false), "stop");
        assert_eq!(resolve_finish_reason(Some("length"), false), "length");
    }

    #[test]
    fn an_upstream_status_is_relayed_rather_than_flattened_into_a_500() {
        let rate_limited = bindings::tachyon::accelerator::cpu::GenerationError {
            message: "upstream returned HTTP 429".to_owned(),
            upstream_status: Some(429),
        };
        let (status, body) = generation_error_payload("coder", rate_limited);
        assert_eq!(status, 429, "a client's backoff depends on seeing the 429");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("error body");
        assert_eq!(payload["error"]["type"], "rate_limit_error");

        // A rejected request is the caller's to fix, not to retry.
        let (status, _) = generation_error_payload(
            "coder",
            bindings::tachyon::accelerator::cpu::GenerationError {
                message: "upstream returned HTTP 400".to_owned(),
                upstream_status: Some(400),
            },
        );
        assert_eq!(status, 400);

        // An upstream credential failure is this node's misconfiguration, so it
        // must not reach the caller as its own authentication problem.
        let (status, _) = generation_error_payload(
            "coder",
            bindings::tachyon::accelerator::cpu::GenerationError {
                message: "upstream returned HTTP 401".to_owned(),
                upstream_status: Some(401),
            },
        );
        assert_eq!(status, 502);

        // A local failure has no remote status to relay and stays a 500.
        let (status, _) = generation_error_payload(
            "coder",
            bindings::tachyon::accelerator::cpu::GenerationError {
                message: "model alias `coder` is not loaded".to_owned(),
                upstream_status: None,
            },
        );
        assert_eq!(status, 500);
    }

    #[test]
    fn streaming_gate_forwards_prose_and_withholds_a_tagged_tool_call() {
        // The prose before the call must reach the client as it is generated —
        // buffering everything would cost time-to-first-token on every request
        // that merely offers tools.
        let (streamed, whole, emitted) = run_gate(
            ToolCallParser::Qwen,
            &[
                "Let me check.",
                "\n<tool_",
                "call>{\"name\":\"search\",\"arguments\":{}}</tool_call>",
            ],
        );
        // Everything up to the opener except the whitespace separating the
        // prose from it, which the buffered parser trims away — a streamed
        // chunk cannot be un-sent, so it is withheld until the gate knows
        // whether a call follows.
        assert_eq!(streamed, "Let me check.");
        // `emitted` still advances past the withheld newline to the opener, so
        // the handler's tail reconciliation does not hand it back.
        assert_eq!(emitted, streamed.len() + 1);
        // The tag itself never leaks into the content stream.
        assert!(!streamed.contains("<tool_call>"));
        assert!(whole.contains("<tool_call>"));
    }

    #[test]
    fn streaming_gate_matches_an_opener_split_across_fragments() {
        // `<tool_` / `call>` arriving separately must still be caught, or the
        // opening tag leaks into the transcript.
        let (streamed, _, _) = run_gate(
            ToolCallParser::Qwen,
            &["hi ", "<tool_", "call>{\"name\":\"f\"}</tool_call>"],
        );
        // The space before the opener is trimmed by the buffered parser, so it
        // is not streamed either.
        assert_eq!(streamed, "hi");
    }

    #[test]
    fn streaming_gate_streams_everything_when_no_tool_call_appears() {
        let (streamed, whole, _) = run_gate(ToolCallParser::Qwen, &["all ", "plain ", "prose"]);
        // The tail is held back until the stream ends; the handler flushes it
        // from the parsed content afterwards.
        assert!(whole.starts_with(&streamed));
        assert_eq!(whole, "all plain prose");
    }

    #[test]
    fn streaming_gate_withholds_an_anchored_json_call_entirely() {
        // A `json` response is a tool call only when the *whole* output is one
        // JSON value, so nothing may be streamed as content.
        let (streamed, whole, emitted) = run_gate(
            ToolCallParser::Json,
            &["{\"tool_calls\":[{\"name\":", "\"search\"}]}"],
        );
        assert!(streamed.is_empty());
        assert_eq!(emitted, 0);
        assert!(whole.starts_with('{'));
    }

    #[test]
    fn streaming_gate_does_not_anchor_on_a_brace_inside_prose() {
        // A `{` mid-sentence cannot start a JSON tool call — the whole output
        // would have to parse — so it must not stop content from streaming.
        let (_, whole, _) = run_gate(ToolCallParser::Json, &["use ", "Vec<T> { .. } ", "here"]);
        let mut gate = StreamingContentGate::new(ToolCallParser::Json);
        gate.push(&whole);
        assert!(!gate.tripped, "a brace inside prose must not trip the gate");
    }

    #[test]
    fn streaming_gate_withholds_the_mistral_marker() {
        let (streamed, _, _) = run_gate(
            ToolCallParser::Mistral,
            &["Checking\n", "[TOOL_CALLS] [{\"name\":\"fetch\"}]"],
        );
        assert_eq!(streamed, "Checking");
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
            stream_options: None,
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
