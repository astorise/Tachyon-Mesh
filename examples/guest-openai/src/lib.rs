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
    /// Wall-clock budget for this generation. Not an OpenAI field, so it was
    /// simply dropped: the host fell back to its own default and a client that
    /// had asked for a shorter leash on an agentic turn waited out the long
    /// one. Accepted here and forwarded; the host still clamps it to its
    /// ceiling.
    #[serde(default)]
    max_generation_ms: Option<u64>,
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
    /// Structured-output request. Declared here because serde drops what this
    /// struct does not name: the host has a constrained decoder and the
    /// upstream backend has a `response_format` translation, and neither was
    /// reachable through the public OpenAI route because the field never
    /// survived deserialization. A client supplying a schema got unconstrained
    /// output and no error — the same failure the `tools` field had.
    #[serde(default)]
    response_format: Option<ResponseFormat>,
}

/// The subset of OpenAI's `response_format` this route acts on.
///
/// `{"type": "text"}` carries no constraint. The other two do: `json_schema`
/// forwards its own `schema` — `name` and `strict` describe the request, not
/// the grammar — and `json_object` asks for *some* valid JSON object without
/// saying which, which is the permissive object schema rather than nothing.
#[derive(Debug, Clone, Default, Deserialize)]
struct ResponseFormat {
    /// Dropping this left `json_object` indistinguishable from `text`, so a
    /// client that asked for JSON got unconstrained prose and no error.
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    json_schema: Option<JsonSchemaFormat>,
}

impl ResponseFormat {
    /// The schema text to constrain decoding with, or why this format cannot be
    /// honoured.
    ///
    /// Every way this route refuses a format is a refusal a client can read.
    /// The one thing it must never do is accept a format and then not apply it:
    /// the caller asked for schema-conforming data, got prose, and has nothing
    /// telling it the difference — so whatever it parses next treats arbitrary
    /// text as structured.
    ///
    /// The `type` is checked before the schema, so a misspelling is a 400 even
    /// when a schema rides along. Within a recognised type an explicit schema
    /// wins, which is what lets `json_object` carry one and be honoured
    /// precisely rather than permissively.
    fn schema_source(&self) -> Result<Option<String>, String> {
        let declared = self
            .json_schema
            .as_ref()
            .and_then(|format| format.schema.as_ref())
            .map(serde_json::Value::to_string);
        match self.kind.as_deref() {
            // The only type that constrains nothing.
            Some("text") => Ok(None),
            // "Any object" is the whole of what `json_object` promises, and it
            // is expressible: the host's compiler reads a schema with no
            // `properties` as accepting every object and nothing else.
            // Forwarding nothing here made the mode a no-op that clients cannot
            // detect, since the failure looks like a model that ignored the
            // instruction.
            Some("json_object") => Ok(Some(
                declared.unwrap_or_else(|| r#"{"type":"object"}"#.to_owned()),
            )),
            Some("json_schema") => declared.map(Some).ok_or_else(|| {
                "`response_format` of type `json_schema` must carry `json_schema.schema`".to_owned()
            }),
            // No `type` at all is not a format; a bare `json_schema` block is
            // still honoured, since that is unambiguous about what was wanted.
            None => Ok(declared),
            // A misspelling — `json-Object`, `json_obect` — used to fall
            // through to unconstrained inference and a 200. The caller stated a
            // format, so answering as though it had not is the same silent
            // no-op the branches above exist to prevent.
            Some(other) => Err(format!(
                "`response_format.type` `{other}` is not supported; use `text`, `json_object`, \
                 or `json_schema`"
            )),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
struct JsonSchemaFormat {
    #[serde(default)]
    schema: Option<serde_json::Value>,
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
    /// Whatever the client sent, re-serialized unchanged into the host
    /// envelope.
    ///
    /// `Option<String>` here narrowed the OpenAI message shape, and the
    /// narrowing was reachable: a standard multimodal turn whose `content` is
    /// an array of parts — `text` plus `image_url` — failed *deserialization*,
    /// so the request never reached the host's opaque `Vec<Value>` forwarding
    /// and a perfectly valid request came back as HTTP 500, even when the
    /// selected `openai:` upstream understands it.
    ///
    /// Kept opaque rather than modelled: the shape belongs to the provider,
    /// and the two local runtimes already refuse structured content with a
    /// typed invalid-request error, which is the right answer for a backend
    /// that genuinely cannot read it.
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    content: serde_json::Value,
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
    /// The pre-`tool_calls` shape of an assistant turn that made a call.
    ///
    /// A client continuing a legacy conversation replays it, and with no field
    /// to land in it was dropped: the upstream then saw a `role: "function"`
    /// result with no request before it, and answered as though it had never
    /// asked. Relayed opaquely, like `tool_calls`, because the provider that
    /// still speaks this dialect is the one that knows its shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    function_call: Option<serde_json::Value>,
    /// A provider's structured safety refusal, carried through from the host.
    ///
    /// OpenAI puts this on its own field precisely so a caller can tell a
    /// refusal from an answer. Without it a refusal reached the client as
    /// `content: null` with `finish_reason: "stop"` — indistinguishable from a
    /// model that returned nothing, so an agent read the empty string as the
    /// reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refusal: Option<String>,
    /// Everything this struct does not name, carried through rather than
    /// dropped. A provider-specific field on an inbound turn is the client's
    /// and the upstream's business, not this route's, and serde discards what
    /// it cannot name — so a field that mattered vanished between the client
    /// and the server that understands it.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    extra: serde_json::Map<String, serde_json::Value>,
}

#[cfg(test)]
impl ChatMessage {
    fn text(role: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: role.into(),
            content: serde_json::Value::String(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
            function_call: None,
            refusal: None,
            extra: serde_json::Map::new(),
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
    /// Mirrors the top-level field, for clients that keep every nonstandard
    /// option in one place.
    #[serde(default)]
    max_generation_ms: Option<u64>,
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
    /// A provider safety refusal, on its own field exactly as OpenAI puts it —
    /// a client that can tell a refusal from an answer only can because the two
    /// never share a field.
    #[serde(skip_serializing_if = "Option::is_none")]
    refusal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<StreamToolCall>>,
}

impl ChunkDelta {
    fn role(role: &'static str) -> Self {
        Self {
            role: Some(role),
            content: None,
            refusal: None,
            tool_calls: None,
        }
    }

    fn content(content: String) -> Self {
        Self {
            role: None,
            content: Some(content),
            refusal: None,
            tool_calls: None,
        }
    }

    fn tool_calls(tool_calls: Vec<StreamToolCall>) -> Self {
        Self {
            role: None,
            content: None,
            refusal: None,
            tool_calls: Some(tool_calls),
        }
    }

    fn empty() -> Self {
        Self {
            role: None,
            content: None,
            refusal: None,
            tool_calls: None,
        }
    }

    fn refusal(refusal: String) -> Self {
        Self {
            role: None,
            content: None,
            refusal: Some(refusal),
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
    /// Set on a reservation row the host writes while a hot reload swaps the
    /// runtime out. The alias is still the manifest's — so nothing may claim it
    /// — but no runtime is answering for it yet, and advertising it would point
    /// clients at a model that 500s until publication catches up.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    withdrawn: bool,
}

/// Marks a registry row the host derived from the sealed manifest.
const REGISTRY_SOURCE_CONFIG: &str = "config";

/// Marker the host puts in its error when it refuses a registry write because a
/// configured binding owns the alias. Matched rather than parsed: WIT gives the
/// table a `result<_, string>`, so a sentinel substring is the only typed
/// channel there is, and the alternative is reporting a 409 as a 500.
const REGISTRY_ALIAS_TAKEN_MARKER: &str = "model-alias-claimed-by-config";

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

/// Whether the registry holds a *reservation* row for `alias`.
///
/// `list_models()` filters withdrawn rows out, which is right for the listing
/// and for resolving a qualified `engine/alias`: both go through the filtered
/// set, so a reserved alias is absent from each. A bare name did not, because
/// the resolver falls back to using the requested name as the alias when it
/// matches nothing — a fallback meant for aliases the registry never knew, not
/// for one it is deliberately holding.
///
/// The effect was a hole exactly the width of a reload: hidden from
/// `GET /ai/v1/models`, still reachable by bare name, and answered by whichever
/// runtime happened to be sealed for the route. Reading the raw row is what
/// tells the two cases apart, since the filtered list cannot.
fn alias_is_withdrawn(alias: &str) -> bool {
    models_table()
        .get(alias)
        .ok()
        .and_then(|raw| serde_json::from_slice::<ModelInfo>(&raw).ok())
        .is_some_and(|info| info.withdrawn)
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
        // The check above is a courtesy — it produces the better message — but
        // it is not what makes this safe: it and this write are two separate
        // host transactions, so a reload publishing the alias in between would
        // slip past it. The host re-tests ownership inside the write and
        // refuses with this marker, which is the answer that actually counts.
        if let Err(error) = models_table().set(&info.alias, &value) {
            if error.contains(REGISTRY_ALIAS_TAKEN_MARKER) {
                return Ok(openai_error_payload(
                    409,
                    format!(
                        "model alias `{}` is claimed by a configured binding in the sealed \
                         manifest; register under a free alias, or declare the binding `dynamic`",
                        info.alias
                    ),
                    "invalid_request_error",
                ));
            }
            return Err(format!("model registry write failed: {error}"));
        }
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
        // Same race, same authority: the host re-tests ownership inside the
        // delete, so an alias published between the check and here is refused
        // there rather than silently removed.
        if let Err(error) = models_table().delete(alias) {
            if error.contains(REGISTRY_ALIAS_TAKEN_MARKER) {
                return Ok(openai_error_payload(
                    409,
                    format!(
                        "model alias `{alias}` is claimed by a configured binding in the sealed \
                         manifest; it is removed by editing the manifest, not by deregistering"
                    ),
                    "invalid_request_error",
                ));
            }
            return Err(format!("model registry delete failed: {error}"));
        }
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
        // Same reservation rule as the chat path. `list_models()` filters a
        // withdrawn row, so a bare embedding name fell through to this
        // fallback and ran against whichever runtime was sealed for the route
        // — for the length of a reload window, or indefinitely after a failed
        // reconciliation. The chat path grew this check; embeddings resolve
        // through the same lookup and needed it too.
        None if !request.model.contains('/') && alias_is_withdrawn(&request.model) => {
            return Ok(openai_error_payload(
                404,
                format!("model `{}` is unavailable", request.model),
                "model_not_found",
            ));
        }
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
    // Checked here rather than at encode time so the refusal is a 400 the
    // client can act on, and so it lands before a model is loaded — a request
    // this route cannot honour should not cost a checkpoint residency.
    if let Some(format) = request.response_format.as_ref() {
        if let Err(detail) = format.schema_source() {
            return Ok(openai_error_payload(400, detail, "invalid_request_error"));
        }
    }

    let models = list_models()?;
    let registered = resolve_registered_model(&request.model, &models);
    let Some(alias) = resolve_requested_alias(&request.model, registered, || {
        alias_is_withdrawn(&request.model)
    }) else {
        return Ok(openai_error_payload(
            404,
            format!("model `{}` is unavailable", request.model),
            "model_not_found",
        ));
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
/// The alias a request names, or `None` when it must be answered with a 404.
///
/// Three cases, and the middle one is the one that was missing:
///
/// * a row in the listing — the registry's own alias, whatever spelling the
///   client used to reach it;
/// * a bare name the registry is *holding* — reserved during a reload swap, so
///   it is unavailable rather than unknown;
/// * a bare name the registry has never heard of — passed through, because an
///   alias can be loadable without having a row yet.
///
/// `reserved` is a closure rather than a `bool` because it reads the raw
/// registry row through the host, and only the second case needs to ask. A
/// request that resolved, or that used a qualified `engine/alias`, decides
/// without the extra round trip.
fn resolve_requested_alias(
    requested: &str,
    registered: Option<&ModelInfo>,
    reserved: impl FnOnce() -> bool,
) -> Option<String> {
    if let Some(model) = registered {
        return Some(model.alias.clone());
    }
    // A qualified name that matched nothing is simply not here: the engine
    // prefix only exists on rows, so there is no bare-alias reading of it.
    if requested.contains('/') {
        return None;
    }
    (!reserved()).then(|| requested.to_owned())
}

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
    // Structured calls are adopted only when the client asked for tools. An
    // upstream that returns `tool_calls` to a request that offered none is
    // answering a question nobody put, and surfacing them would hand a client
    // that never opted in a `message.tool_calls` array plus a `tool_calls`
    // finish reason — a response shape it has no code path for.
    let parsed = if completed.tool_calls.is_empty() || !request.has_tool_intent() {
        parse_assistant_output(&request, &completed.text)
    } else {
        ParsedAssistantOutput {
            content: completed.text.clone(),
            tool_calls: adopt_host_tool_calls(completed.tool_calls),
        }
    };
    // Checked after both sources have been reconciled, so the rule covers a
    // call the host reported as a field and one recovered from the text alike:
    // either way it is an instruction the client has no code for.
    if let Some(name) = request.unoffered_call(&parsed.tool_calls) {
        return Ok(openai_error_payload(
            502,
            format!(
                "model `{}` returned a call to `{name}`, which this request did not offer",
                request.model
            ),
            "server_error",
        ));
    }
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
                    serde_json::Value::Null
                } else {
                    serde_json::Value::String(parsed.content)
                },
                tool_calls: parsed.tool_calls,
                // Response-side only: this node answers in the modern shape,
                // and an assistant turn never carries the other three.
                tool_call_id: None,
                name: None,
                function_call: None,
                // Carried through untouched. A refusal is the provider's, and
                // folding it into `content` would be an agent parsing "I can't
                // help with that" as data.
                refusal: completed.refusal,
                // Nothing to carry on the way out: this node composes the
                // assistant turn itself.
                extra: serde_json::Map::new(),
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
            // A refusal is not content and never goes through the tool-call
            // gate: it cannot contain a call, and holding it back would delay
            // the one part of the answer the client most needs promptly.
            Ok(Some(bindings::tachyon::accelerator::cpu::StreamEvent::Refusal(refusal))) => {
                let chunk = ChatCompletionChunk {
                    usage: None,
                    id: id.clone(),
                    object: "chat.completion.chunk",
                    created,
                    model: request.model.clone(),
                    choices: vec![ChunkChoice {
                        index: 0,
                        delta: ChunkDelta::refusal(refusal),
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
    // Gated on intent, like the buffered branch. An upstream that returns
    // structured calls to a request that offered no tools — or set
    // `tool_choice: "none"` — is answering a question the client did not ask,
    // and adopting them here exposed a dispatchable `tool_calls` delta the
    // caller had explicitly forbidden. The buffered path grew this guard; the
    // streamed one adopted unconditionally.
    let mut tool_calls = if request.has_tool_intent() {
        adopt_host_tool_calls(host_tool_calls)
    } else {
        Vec::new()
    };
    if let Some(gate) = gate {
        let (whole, sent, overflowed) = gate.finish();
        // The gate had to discard a fragment to stay within its retention
        // cap, so `whole` is not the generation — it is missing bytes from
        // somewhere in the middle. Parsing it anyway risks handing back a
        // `tool_calls` delta built from a truncated argument list, or dropping
        // a call the buffered parser can no longer find its closing tag for.
        // The status line already went out with the headers, so this is
        // reported the same way every other post-hoc failure on this path is:
        // an SSE error frame in place of the completion.
        if overflowed {
            write_sse_error(
                &writer,
                &request.model,
                bindings::tachyon::accelerator::cpu::GenerationError {
                    message: "the response exceeded the tool-call gate's retention limit before it could be parsed".to_owned(),
                    upstream_status: Some(502),
                    invalid_request: false,
                },
            )?;
            return Ok((200, Vec::new()));
        }
        let parsed = parse_assistant_output(&request, &whole);

        // Everything the client has not received yet. `sent` is what actually
        // went out, so this is also where the gate's deferred decisions get
        // settled — the separator it dropped before a tool call comes back if
        // the buffered parse kept it, and stays dropped if it did not.
        //
        // Two reconciliations, because the two cases differ.
        //
        // With no tool calls, the parser returned the raw text unchanged, so
        // the exact byte cut is right. This is the case that must not trim:
        // a response opening with whitespace trips the anchored JSON gate at
        // its first `{`, sending nothing, and cutting the tail anywhere but 0
        // would drop the whitespace the buffered response keeps.
        //
        // With tool calls, `parsed.content` is the text minus those regions,
        // trimmed at both ends. Only its *leading* trim shifts the offset of
        // what was sent, so the consumed length is the sent prefix minus the
        // whitespace the parser dropped from the front. Trimming that prefix at
        // both ends instead would hand its own trailing whitespace back as new
        // content — three newlines where the buffered parser produces two.
        //
        // Which of the two applies turns on whether the *parse* is what
        // supplies the calls, not on whether it found any. When the host
        // already reported structured calls they win outright, so the parse is
        // discarded whole — and its content has to be discarded with it.
        // Keeping only its opinion of the text cut out the regions it mistook
        // for calls, so a response carrying both a real structured call and
        // prose that merely looks like one lost that prose, silently, on the
        // stream but not in the buffered reply.
        let tail = unsent_tail(
            &whole,
            sent,
            &parsed,
            /* host_supplied_calls */ !tool_calls.is_empty(),
        );
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

    // The same rule as the buffered path, on the only channel left: the status
    // line went out with the headers, so an unusable call is reported as an
    // error frame rather than emitted as a dispatchable delta.
    if let Some(name) = request.unoffered_call(&tool_calls) {
        write_sse_error(
            &writer,
            &request.model,
            bindings::tachyon::accelerator::cpu::GenerationError {
                message: format!("returned a call to `{name}`, which this request did not offer"),
                upstream_status: Some(502),
                invalid_request: false,
            },
        )?;
        return Ok((200, Vec::new()));
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
///
/// `seen` does not retain the whole generation. Once a byte range is proven
/// opener-free *and* has actually been sent downstream, it is dropped down to
/// a one-byte anchor (see the note on `push`'s unanchored branch for why the
/// anchor has to stay) — `dropped` records how much, so `emitted`/`sent` can
/// stay absolute offsets into the conceptual whole stream while indexing into
/// `seen` at `offset - dropped`. That keeps `seen` at O(`hold`) for the common
/// case of a long answer that never opens a call, which also fixes the
/// quadratic rescanning `find_opener` used to do: it scans all of `seen`, and
/// `seen` no longer grows without bound.
///
/// An anchored gate gets a stronger version of the same idea: once its leading
/// bytes diverge from every opener, no later byte can retroactively open a
/// call (the anchor is fixed at the start), so the gate is *decided* and never
/// needs to buffer again — every further fragment is forwarded immediately,
/// the same as if no gate existed.
///
/// What's left after both of those — the region from an opener to the end of
/// the stream, and an anchored gate still undecided because the answer is
/// nothing but whitespace so far — is capped at `MAX_GATED_RETENTION`. A
/// hostile upstream that opens a call and never closes it, or never emits a
/// non-whitespace byte, cannot pin unbounded memory per stream; hitting the
/// cap sets `overflowed`, and the caller is expected to fail the completion
/// rather than hand back content parsed from a buffer that was silently cut
/// short.
struct StreamingContentGate {
    openers: &'static [&'static str],
    anchored: bool,
    hold: usize,
    seen: String,
    /// Absolute offset of `seen[0]` into the conceptual whole stream. Bytes
    /// below this were proven opener-free and already sent, so they no longer
    /// need to be retained.
    dropped: usize,
    /// How far into the whole stream the gate has *accounted* for content —
    /// the scan position. Advances past whitespace it deliberately withheld.
    emitted: usize,
    /// How far into the whole stream the gate has actually *sent* content.
    /// Never ahead of `emitted`, and behind it exactly by the separator
    /// dropped before a tool call. The two were one counter, which silently
    /// made "withheld" mean "delivered": the caller's tail reconciliation then
    /// started after whitespace nobody had received, so a response opening
    /// with whitespace lost it, and prose *after* a call lost the separator
    /// before it.
    sent: usize,
    tripped: bool,
    /// Set once an anchored gate's leading bytes have diverged from every
    /// opener. From here on nothing can ever open a call, so nothing is
    /// buffered — every fragment is forwarded as-is.
    decided_no_call: bool,
    /// Set once a fragment had to be discarded to stay within
    /// `MAX_GATED_RETENTION`. The retained text is no longer a faithful copy
    /// of the generation from that point on, so the caller must not treat
    /// whatever `finish()` returns as a real completion.
    overflowed: bool,
    /// Absolute position of the earliest byte not yet proven walled off from
    /// every later byte — `None` when nothing is pinning the drop boundary.
    /// Fixed at the position it was first set to for as long as it stays
    /// `Some`: extending *how far* the search for a wall has to look (see
    /// `protected_until`) must never be confused with moving *this* floor
    /// forward, or bytes between the original pin and wherever the search
    /// got to would be dropped despite the risk they were pinned against
    /// still being unresolved. See the long note in `push`'s unanchored
    /// branch for what "walled off" means.
    drop_ceiling: Option<usize>,
    /// Absolute position such that a wall search has confirmed no wall
    /// exists in `[drop_ceiling, protected_until)` — every byte in it is
    /// either the original pin or a later tag-start byte whose own
    /// resolution window is still being chased. Meaningless while
    /// `drop_ceiling` is `None`.
    protected_until: usize,
    /// Absolute position the wall search has scanned to without yet finding
    /// either a wall or a reason to extend `protected_until` further —
    /// tracked separately so each round only examines newly-arrived bytes
    /// instead of re-scanning `[drop_ceiling, protected_until)` from
    /// scratch. Meaningless while `drop_ceiling` is `None`.
    scan_cursor: usize,
    /// The leading byte of every tag this parser recognizes, open or close —
    /// every tagged dialect's pair shares one (`<`). Empty for a parser whose
    /// buffered parse cannot rejoin bytes a drop has separated — Mistral's
    /// single-marker split never rescans or mutates, so `drop_ceiling` has
    /// nothing to protect against for it — and for an anchored gate, which
    /// never runs the drop logic this guards at all.
    tag_start_chars: Vec<char>,
}

/// Ceiling on what a single gate will retain once it can no longer bound
/// itself the cheap way — a tripped gate holding a call for the buffered
/// parser, an anchored gate that has seen nothing but whitespace so far, or a
/// separator ahead of a still-possible opener that is too long to resolve
/// yet. Without it, a stream built out of any of those would retain up to the
/// whole per-stream cap; the node admits many concurrent streams, so an
/// unbounded gate is an unbounded multiple of that.
const MAX_GATED_RETENTION: usize = 256 * 1024;

/// Granularity `push` slices a large fragment into before running it through
/// the ordinary per-push resolve logic. Large enough that a realistic
/// fragment (SSE frames are accepted up to 1 MiB) needs only a handful of
/// iterations; comfortably above the widest UTF-8 character (4 bytes), which
/// `push`'s slicing loop relies on to always make progress.
const PUSH_CHUNK_SIZE: usize = 64 * 1024;

impl StreamingContentGate {
    fn new(parser: ToolCallParser) -> Self {
        let (openers, anchored) = tool_call_openers(parser);
        let hold = openers
            .iter()
            .map(|opener| opener.len())
            .max()
            .unwrap_or(1)
            .saturating_sub(1);
        // `drop_ceiling` exists to protect against `parse_tagged_tool_calls`'s
        // iterative, mutating tag removal splicing distant bytes together —
        // that rewriting only happens for the tagged dialects. Mistral's
        // parser does one split on a single marker and never rescans or
        // mutates, so it has nothing for a dropped prefix to rejoin with; an
        // incidental `[` from ordinary code (`items[0]`) is exactly the kind
        // of realistic content this must not needlessly pin retention on.
        let mut tag_start_chars: Vec<char> =
            if matches!(parser, ToolCallParser::Qwen | ToolCallParser::QwenCoder) {
                // Every dialect's closing tag shares its opener's leading byte
                // (`</tool_call>` vs `<tool_call>`), so the openers alone are
                // enough to derive it without hard-coding the closers here too.
                openers.iter().filter_map(|o| o.chars().next()).collect()
            } else {
                Vec::new()
            };
        tag_start_chars.sort_unstable();
        tag_start_chars.dedup();
        Self {
            openers,
            anchored,
            hold,
            seen: String::new(),
            dropped: 0,
            emitted: 0,
            sent: 0,
            tripped: false,
            decided_no_call: false,
            overflowed: false,
            drop_ceiling: None,
            protected_until: 0,
            scan_cursor: 0,
            tag_start_chars,
        }
    }

    /// Absorb a fragment, returning the content safe to stream right now.
    ///
    /// A single upstream fragment is not necessarily small: SSE frames up to
    /// 1 MiB are accepted, and some backends emit the whole generation as one
    /// fragment. Handing all of it to [`Self::push_chunk`] in one call would
    /// grow `seen` to the fragment's full size before that call ever gets a
    /// chance to find an opener or release safe content — tripping
    /// `MAX_GATED_RETENTION` on a fragment that is, in fact, fully
    /// resolvable (most of it content, with no call anywhere in it). So a
    /// large fragment is walked in bounded slices instead, each one run
    /// through the ordinary per-push logic; that keeps `seen` from growing
    /// past what driving the same bytes in through several smaller pushes
    /// would have needed, which is the shape the retention cap is actually
    /// meant to guard: a call that stays open, or an anchored prefix that
    /// stays undecided, past what a single fragment could ever explain.
    fn push(&mut self, fragment: &str) -> Option<String> {
        if fragment.len() <= PUSH_CHUNK_SIZE {
            return self.push_chunk(fragment);
        }
        let mut streamed = String::new();
        let mut rest = fragment;
        while !rest.is_empty() {
            // Once the gate can no longer resolve anything incrementally —
            // tripped, or decided against ever tripping — slicing further
            // buys nothing: a tripped gate's `push_chunk` just forwards to
            // `absorb_capped`, which already caps on its own, and a decided
            // gate forwards every fragment untouched. Handing over what's
            // left in one call is equivalent and skips the bookkeeping.
            if self.tripped || self.decided_no_call {
                if let Some(content) = self.push_chunk(rest) {
                    streamed.push_str(&content);
                }
                break;
            }
            let at = floor_char_boundary(rest, PUSH_CHUNK_SIZE.min(rest.len()));
            // Always > 0 as long as `PUSH_CHUNK_SIZE` is at least as wide as
            // the longest UTF-8 character (4 bytes): `floor_char_boundary`
            // only walks backward to find one, and a character is never more
            // than 4 bytes wide, so it cannot walk past a chunk size that
            // size back to 0. Asserted rather than silently guarded, so a
            // future change to the constant fails loudly instead of looping
            // forever.
            debug_assert!(at > 0, "PUSH_CHUNK_SIZE must be at least 4 bytes");
            let (chunk, remainder) = rest.split_at(at);
            if let Some(content) = self.push_chunk(chunk) {
                streamed.push_str(&content);
            }
            rest = remainder;
        }
        (!streamed.is_empty()).then_some(streamed)
    }

    fn push_chunk(&mut self, fragment: &str) -> Option<String> {
        // Decided: the anchor can never be reached again, so nothing can open
        // a call. Nothing to buffer for — pass the fragment straight through.
        if self.decided_no_call {
            return Some(fragment.to_owned());
        }
        if self.tripped {
            self.absorb_capped(fragment);
            return None;
        }

        self.absorb_capped(fragment);

        if let Some(at) = self.find_opener() {
            self.tripped = true;
            let local_emitted = self.emitted - self.dropped;
            // Everything before the opener is genuine content — minus the
            // whitespace that separates it from the call. The buffered parser
            // removes the call region and `trim()`s what is left, so emitting
            // the newline in `Let me check.\n<tool_call>…` would leave the
            // concatenated deltas differing from the buffered message by
            // exactly that whitespace, and the streaming contract is that the
            // two are equal.
            //
            // Unless a pin is still open: this opener completing does not by
            // itself prove the *prefix* before it survives the buffered
            // parse's own rewriting unchanged — `<tool_calls` immediately
            // ahead of exactly this kind of complete opener is the case that
            // rejoins into a tag that was never literally in the response
            // (see the long note below in the unanchored branch). Once
            // tripped there is no later round left to reconsider a
            // streaming decision, so the prefix stays fully withheld here
            // rather than only partly, the same as a message that opens a
            // call from its very first byte.
            let content = if self.drop_ceiling.is_some() {
                String::new()
            } else {
                self.seen[local_emitted..at].trim_end().to_owned()
            };
            // `emitted` still advances to the opener — the whitespace is
            // accounted for, so nothing rescans it — but `sent` stops at what
            // actually went out. Whether that separator is dropped or restored
            // is not knowable yet: it is internal whitespace if prose follows
            // the call, and a trailing edge the buffered parser trims if not.
            // The caller decides once it has parsed the whole text.
            self.sent = self.emitted + content.len();
            self.emitted = self.dropped + at;
            return (!content.is_empty()).then_some(content);
        }

        if self.anchored {
            // Anchored openers only count at the start, so a prefix that is
            // no longer a prefix of any opener can never become one, however
            // much more text arrives. Once that's true the gate is decided:
            // flush what accumulated while deciding, and stop accumulating
            // for the rest of the stream — the cheap win for the JSON parser,
            // whose gate would otherwise retain the whole answer just to keep
            // re-checking a question already settled.
            let trimmed = self.seen.trim_start();
            if !trimmed.is_empty() && !self.openers.iter().any(|o| o.starts_with(trimmed)) {
                self.decided_no_call = true;
                let content = std::mem::take(&mut self.seen);
                let len = content.len();
                self.dropped += len;
                self.emitted += len;
                self.sent += len;
                return Some(content);
            }
            return None;
        }

        let ceiling = floor_char_boundary(&self.seen, self.seen.len().saturating_sub(self.hold));
        // Trailing whitespace is withheld for the same reason, before we know
        // whether an opener follows it. Nothing is lost when none does: the
        // buffered parse then returns the text unchanged, and the caller emits
        // whatever it kept beyond what was streamed.
        let raw_safe = self.seen[..ceiling].trim_end().len();
        let local_emitted = self.emitted - self.dropped;
        if raw_safe <= local_emitted {
            return None;
        }

        // `parse_tagged_tool_calls` does more than trim: it mutates as it
        // scans, cutting out each matched region and rejoining what was on
        // either side of it. Two bytes that were nowhere near each other in
        // the response can end up adjacent after an earlier cut removes
        // everything between them — `<tool_calls` immediately followed by a
        // well-formed `<tool_call>…</tool_call>` rejoins, after that inner
        // match is excised, into a `<tool_calls>` that was never literally in
        // the text. A prefix like that can end up meaning something entirely
        // different once the full response is seen — sometimes swallowed
        // into a tool call's own payload, sometimes erased outright — so it
        // must not be *streamed* any more than it may be dropped: once a
        // byte like this has gone out over SSE there is no taking it back,
        // regardless of what `finish()` later reconstructs internally.
        // `find_opener` only asks "is a complete opener present anywhere in
        // `seen`", which says nothing about a malformed prefix like
        // `<tool_calls` — it will never complete into an opener on its own,
        // so nothing stops it from otherwise being marked `safe` to send.
        //
        // There is no bound on how far ahead the match that would complete
        // such a rewrite might be, so the only sound rule is to never emit —
        // as content, or into `seen` for the buffered parser to reconsider —
        // past the first byte that could start any of this parser's tags,
        // open or close (they all share the same leading byte for every
        // dialect in use).
        //
        // That pin does not have to last forever: the risk requires an
        // *unbroken* run of tag-start bytes connecting the pinned position to
        // whatever match eventually gets removed, and a byte that is provably
        // outside every such run is a wall — it can never be part of a
        // removed span (removal only ever targets a matched tag, which
        // starts with one of `tag_start_chars`), so nothing later can ever
        // splice across it back to the pin. Once a wall is found anywhere,
        // `drop_ceiling` clears and both streaming and dropping resume from
        // scratch.
        //
        // Finding one takes more than checking the very next byte, though: a
        // false start like `<tool_calls` is packed with ordinary,
        // non-tag-start characters (`t`, `o`, `o`, `l`, …) that prove
        // nothing on their own — they are still part of the very attempt
        // being resolved, and a tag-start byte immediately after it (a
        // second false start, or the real match this whole thing exists to
        // protect) extends how far the run reaches before a wall can be
        // confirmed. `protected_until` tracks that reach; `drop_ceiling`
        // itself never moves until a wall is actually found, however far the
        // search has to extend to find one — moving it early, to wherever
        // the run currently reaches, would let bytes between the original
        // pin and there be streamed or dropped while the very risk they were
        // pinned against is still unresolved.
        let mut search_ceiling_from = 0;
        if let Some(dc) = self.drop_ceiling {
            if self.protected_until < dc + self.hold + 1 {
                self.protected_until = dc + self.hold + 1;
                self.scan_cursor = dc + 1;
            }
            loop {
                // Checked before rounding up to a char boundary: rounding
                // first and comparing after could clamp a target beyond what
                // has actually arrived down to exactly `seen.len()`, which
                // would then read as "enough data" and resolve early.
                let target_local = self.protected_until - self.dropped;
                if target_local > self.seen.len() {
                    break; // not enough data yet to finish resolving the run
                }
                let scan_end = ceil_char_boundary(&self.seen, target_local);
                let scan_start = self.scan_cursor - self.dropped;
                match self.seen[scan_start..scan_end]
                    .find(|c: char| self.tag_start_chars.contains(&c))
                {
                    Some(rel) => {
                        let found = self.dropped + scan_start + rel;
                        self.protected_until = self.protected_until.max(found + self.hold + 1);
                        self.scan_cursor = found + 1;
                    }
                    None => {
                        self.drop_ceiling = None;
                        search_ceiling_from = scan_end;
                        break;
                    }
                }
            }
        }
        // A byte found by the wall search above is not yet physically gone
        // from `seen`, so re-scanning for a *new* pin from position 0 right
        // after clearing one would find that same byte again and immediately
        // re-pin it — detection starts from wherever the wall search reached
        // instead; it stays 0 when there was nothing to clear. It searches up
        // to `raw_safe`, not just the anchor-trimmed prefix that ends up
        // droppable: anything in `[local_emitted, raw_safe)` is a candidate
        // to be *streamed* this round, and every one of those bytes needs the
        // same protection as what gets dropped.
        if self.drop_ceiling.is_none() && search_ceiling_from < raw_safe {
            if let Some(at) = self.seen[search_ceiling_from..raw_safe]
                .find(|c: char| self.tag_start_chars.contains(&c))
            {
                let dc = self.dropped + search_ceiling_from + at;
                self.drop_ceiling = Some(dc);
                self.protected_until = dc + self.hold + 1;
                self.scan_cursor = dc + 1;
            }
        }

        // With any pin now resolved as far as this round's data allows, the
        // actual safe boundary — for streaming *and* for dropping alike — is
        // capped at it: nothing at or past an unresolved pin may be emitted
        // either way, only what precedes it. Trimmed again here, the same
        // way `raw_safe` was: a pin sitting right after whitespace (`"Hello
        // <tool_calls"`) must not commit that whitespace to the client
        // either, since it is exactly the separator-before-a-call case the
        // trailing-whitespace withholding above exists for — the pin is just
        // another kind of thing that might turn out to be an opener.
        let safe = match self.drop_ceiling {
            Some(dc) => {
                let capped = dc - self.dropped;
                local_emitted + self.seen[local_emitted..capped].trim_end().len()
            }
            None => raw_safe,
        };
        if safe <= local_emitted {
            return None;
        }

        let content = self.seen[local_emitted..safe].to_owned();
        self.emitted = self.dropped + safe;
        self.sent = self.emitted;

        // Drop everything below `safe` except its very last byte, which is
        // kept as a permanent one-byte anchor.
        //
        // Without it: if a later push's opener starts exactly at `safe` (no
        // separator in between, or a separator that was itself dropped by an
        // earlier round), the retained text ends up beginning exactly where
        // the tag region will later be excised. The buffered parser's own
        // `trim()` cannot tell that apart from the true start of the
        // message — it would strip whatever follows the removed tag as
        // leading whitespace, even though in the full response it is
        // internal (the separator before the call, or the gap between the
        // closing tag and trailing prose). That is not limited to a
        // whitespace boundary at `safe`: content ending in a non-whitespace
        // byte right before the opener has exactly the same failure mode
        // once that byte is dropped.
        //
        // One retained byte of real content fixes both, because `trim()`
        // only ever touches the outer edges of the string it is given: with
        // a non-whitespace byte in front, nothing after the tag can be
        // mistaken for a leading edge, however far away it is or how much
        // whitespace it starts with. `safe` is the length of a string
        // `trim_end`-ed to it when `drop_ceiling` is `None`, so the anchor
        // character is guaranteed non-whitespace in that case; when a pin
        // capped `safe` instead, the anchor is that pin's own tag-start byte,
        // also never whitespace. Either way no separate check is needed. It
        // is not necessarily one *byte*, though: `safe - 1` can land inside a
        // multibyte character, which `drain` would panic on, so the drop
        // stops at that character's start rather than exactly one byte short.
        let drop_amount = floor_char_boundary(&self.seen, safe - 1);
        self.seen.drain(..drop_amount);
        self.dropped += drop_amount;
        Some(content)
    }

    /// Append `fragment` to `seen`, discarding whatever would push it past
    /// the effective cap and flagging [`Self::overflowed`] when that happens.
    /// Reachable while tripped and while an anchored decision is still
    /// pending — states that can otherwise retain the entire remaining
    /// stream — and, with a taller cap, while a `drop_ceiling` pin is still
    /// being resolved.
    ///
    /// A pin gets the extra room because resolving it is not like the other
    /// two: a tripped call or an undecided anchored prefix stay exactly as
    /// large as whatever gets appended, but a resolved pin collapses back
    /// to the ordinary bound on the very same round it clears (the drop
    /// logic right above this call already handles it — nothing further is
    /// needed here). Capping the fragment *before* that resolution ever runs
    /// would refuse it the one thing it needs to clear: the bytes that would
    /// have proven the wall. A single stray tag-start byte followed by dense
    /// nested-generic code (`Vec<Box<dyn Trait>>>`, chained closely enough
    /// to keep extending the pin) is a realistic way to stay unresolved for
    /// a while without ever containing a call — the extra headroom is sized
    /// to one more slice of whatever a single fragment can add, not to be
    /// unbounded: a pin that still has not resolved by the time even that is
    /// exhausted is treated the same as any other overflow.
    fn absorb_capped(&mut self, fragment: &str) {
        let cap = if self.drop_ceiling.is_some() {
            MAX_GATED_RETENTION + PUSH_CHUNK_SIZE
        } else {
            MAX_GATED_RETENTION
        };
        if self.seen.len() >= cap {
            if !fragment.is_empty() {
                self.overflowed = true;
            }
            return;
        }
        let room = cap - self.seen.len();
        let take = floor_char_boundary(fragment, room.min(fragment.len()));
        self.seen.push_str(&fragment[..take]);
        if take < fragment.len() {
            self.overflowed = true;
        }
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

    /// The retained tail of the generation, for the buffered parser to work
    /// on; how much of *that* the client has actually received; and whether
    /// the retention cap ever discarded a fragment, in which case the other
    /// two values are not a faithful record of the generation and the caller
    /// must not treat them as one.
    ///
    /// Everything dropped that was *not* an overflow was proven opener-free
    /// and already sent in full, so the caller never needs it back:
    /// `parse_assistant_output` run on the retained text alone still finds
    /// every call (none of them were in the dropped prefix), and its own
    /// `trim()` at the retained text's edge cancels out against the same
    /// `trim_start()` the tail reconciliation applies to `whole[..sent]` —
    /// both are computed from the same truncated text, so the offsets stay
    /// consistent even though neither equals what the untruncated generation
    /// would have produced.
    fn finish(self) -> (String, usize, bool) {
        let sent = self.sent - self.dropped;
        (self.seen, sent, self.overflowed)
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

/// Smallest index `idx` that is a char boundary of `text`, no smaller than
/// the given one. The `floor` counterpart above rounds a byte count down to
/// stay inside a bound; this is for the opposite case — a byte count derived
/// from an arbitrary length (like `hold`) that has to examine *at least*
/// that many bytes, so rounding down would under-count.
fn ceil_char_boundary(text: &str, mut idx: usize) -> usize {
    if idx >= text.len() {
        return text.len();
    }
    while !text.is_char_boundary(idx) {
        idx += 1;
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
/// The content the stream still owes, once the gate has sent `sent` bytes.
///
/// Two reconciliations, because the two cases differ.
///
/// When the parse is what supplies the calls, `parsed.content` is the text
/// minus those regions, trimmed at both ends. Only its *leading* trim shifts
/// the offset of what was sent, so the consumed length is the sent prefix minus
/// the whitespace the parser dropped from the front. Trimming that prefix at
/// both ends instead would hand its own trailing whitespace back as new content
/// — three newlines where the buffered parser produces two.
///
/// Otherwise the raw text is the truth and the exact byte cut is right. That
/// covers a response with no calls at all, and — the case this used to get
/// wrong — one where the *host* reported structured calls. Those win outright,
/// so the parse is discarded whole; keeping only its opinion of the text cut
/// out the regions it mistook for calls, and a response carrying both a real
/// structured call and prose that merely looks like one lost that prose on the
/// stream while the buffered reply kept it.
fn unsent_tail<'a>(
    whole: &'a str,
    sent: usize,
    parsed: &'a ParsedAssistantOutput,
    host_supplied_calls: bool,
) -> &'a str {
    if host_supplied_calls || parsed.tool_calls.is_empty() {
        return whole.get(sent..).unwrap_or_default();
    }
    let consumed = whole.get(..sent).unwrap_or_default().trim_start().len();
    parsed.content.get(consumed..).unwrap_or_default()
}

fn resolve_finish_reason(host_reported: Option<&str>, has_tool_calls: bool) -> &'static str {
    match host_reported {
        Some("length") => "length",
        Some("content_filter") => "content_filter",
        _ if has_tool_calls => "tool_calls",
        // Reported by the provider but with nothing behind it. Passing it on
        // tells the client to go read `message.tool_calls` and dispatch what it
        // finds there, and it will find an empty array — so the turn ends with
        // an agent waiting on a call that was never made. `stop` is the honest
        // description of a response that carries only text.
        Some("tool_calls") => "stop",
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
    ///
    /// `tool_choice: "none"` is an offer withdrawn. The OpenAI schema uses it
    /// to say "you may see the tools, do not call them", and a nonempty `tools`
    /// list alongside it is the normal way to send that — so reading intent
    /// from the list alone enabled the parser for exactly the request that
    /// asked it not to. A local model's tool-shaped prose then became
    /// dispatchable calls against a client that had ruled them out.
    fn has_tool_intent(&self) -> bool {
        if self.tool_choice_is_none() {
            return false;
        }
        !self.tools.is_empty() || self.tool_choice.is_some()
    }

    /// The function names this request actually advertised.
    ///
    /// `tool_choice` contributes its own named function: a request may pin a
    /// call without repeating the list, and `has_tool_intent` already treats
    /// that as an offer.
    ///
    /// Empty means *nothing readable was offered*, which is not the same as
    /// "no tools": a client can send entries this side cannot parse a name out
    /// of. Validation is skipped in that case rather than refusing everything,
    /// because a set we failed to read is not evidence about what was in it.
    fn offered_tool_names(&self) -> Vec<&str> {
        fn named(value: &serde_json::Value) -> Option<&str> {
            value.get("function")?.get("name")?.as_str()
        }
        // A pinned choice is the *whole* set, not an addition to it. Unioning
        // it with every advertised name meant a request pinned to `read_file`
        // still accepted a `delete_file` call the backend invented — the
        // opposite of what pinning asks for, and the more dangerous direction.
        if let Some(pinned) = self.tool_choice.as_ref().and_then(|choice| named(choice)) {
            return vec![pinned];
        }
        self.tools.iter().filter_map(|tool| named(tool)).collect()
    }

    /// The first call naming a function this request never offered.
    ///
    /// A backend answering with a call outside the advertised set is answering
    /// a question nobody put — the same rule the intent gate applies to a
    /// request that offered no tools at all, one entry at a time. Passing it on
    /// hands the client a dispatchable `type: "function"` call it has no code
    /// for, and on an `openai:` binding the backend that produced it is a
    /// third-party server the *operator* chose, not the client.
    fn unoffered_call<'c>(&self, calls: &'c [ToolCall]) -> Option<&'c str> {
        let offered = self.offered_tool_names();
        if offered.is_empty() {
            return None;
        }
        calls
            .iter()
            .map(|call| call.function.name.as_str())
            .find(|name| !offered.contains(name))
    }

    /// Whether `tool_choice` is the literal `"none"`.
    ///
    /// Only the string form: `{"type":"function",…}` names a call the client
    /// wants, and any other shape is not a withdrawal this side should infer.
    fn tool_choice_is_none(&self) -> bool {
        self.tool_choice
            .as_ref()
            .and_then(serde_json::Value::as_str)
            .is_some_and(|choice| choice == "none")
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
    if let Some(budget) = request.max_generation_ms.or_else(|| {
        request
            .extra_body
            .as_ref()
            .and_then(|body| body.max_generation_ms)
    }) {
        object.insert("max_generation_ms".to_owned(), serde_json::json!(budget));
    }
    if let Some(stop) = request.stop.clone() {
        object.insert("stop".to_owned(), serde_json::json!(stop.into_vec()));
    }
    // Carried so an `openai:` backend can ask its own provider for streamed
    // usage. Providers emit it only when the option is present, so without this
    // the host had nothing to read and the trailing usage chunk this client
    // explicitly requested never arrived. Sent as the intent, not as the
    // upstream's field: a local backend measures usage regardless, and the
    // upstream runtime is the only place that knows whether the option is safe
    // to forward.
    if request
        .stream_options
        .as_ref()
        .is_some_and(|options| options.include_usage)
    {
        object.insert("include_usage".to_owned(), serde_json::json!(true));
    }
    // The host takes the schema as source text, which is what its grammar
    // compiler parses. `text` is the only format that forwards nothing.
    if let Some(schema) = request
        .response_format
        .as_ref()
        .map(ResponseFormat::schema_source)
        .transpose()?
        .flatten()
    {
        object.insert("json_schema".to_owned(), serde_json::Value::String(schema));
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
        // A reserved alias is absent, not available. This is both the listing
        // and the resolver, so the alias 404s for the length of the swap and
        // comes back when the incoming binding publishes — a client's retry,
        // rather than a row that lies about where a prompt goes.
        .filter(|info| !info.withdrawn)
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
///
/// A request the *host* rejected has no upstream status to relay and is still
/// the caller's to fix — a budget above the binding's ceiling, a prompt that
/// cannot fit the context window. It carries `invalid_request` instead, because
/// without it those became a retryable 500 and the client retried, forever, a
/// request that could never succeed.
fn generation_error_payload(
    model: &str,
    error: bindings::tachyon::accelerator::cpu::GenerationError,
) -> (u16, Vec<u8>) {
    let message = format!("inference failed for model `{model}`: {}", error.message);
    let (status, kind) = match error.upstream_status {
        Some(429) => (429, "rate_limit_error"),
        // The provider rejected the request we forwarded, which for these
        // reflects the caller's own parameters — a tool schema it will not
        // accept, a context overflow, a conflicting field.
        Some(400 | 409 | 413 | 422) => (400, "invalid_request_error"),
        // Not the caller's. The upstream model name and the endpoint path both
        // come from the binding; a client can choose the mesh alias and nothing
        // else. Telling it `invalid_request_error` sends it looking for a
        // mistake in a request it cannot change, while the real fault — a
        // misconfigured `?model=` or base URL — goes unreported.
        Some(404 | 405) => (502, "server_error"),
        Some(status @ 502..=504) => (status, "server_error"),
        Some(_) => (502, "server_error"),
        None if error.invalid_request => (400, "invalid_request_error"),
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
            max_generation_ms: None,
            tool_call_parser: None,
            extra_body: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            seed: None,
            stop: None,
            stream: None,
            stream_options: None,
            response_format: None,
        }
    }

    /// A reserved alias is unavailable, not a free name to load.
    ///
    /// `list_models()` filters withdrawn rows, so a reserved alias is absent
    /// from the listing and from `resolve_registered_model`. The bare-name
    /// fallback then read that absence as "the registry never heard of this"
    /// and used the name as an alias — which, for the length of a reload swap,
    /// ran the request against whichever runtime was sealed for the route
    /// while `GET /ai/v1/models` reported the model as gone.
    ///
    /// The reservation lookup is injected here because it reads the raw row
    /// through the host; the decision it feeds is what this pins down.
    #[test]
    fn a_reserved_bare_alias_is_unavailable_rather_than_loadable() {
        // Reserved: hidden from the listing, held by the registry.
        assert_eq!(
            resolve_requested_alias("qwen-coder", None, || true),
            None,
            "an alias the registry is holding must 404, not fall through"
        );
        // Unknown: hidden from the listing because it has no row at all.
        assert_eq!(
            resolve_requested_alias("qwen-coder", None, || false),
            Some("qwen-coder".to_owned()),
            "a name with no row stays loadable: rows are not a precondition"
        );
        // Qualified and unmatched: no bare-alias reading exists for it, and the
        // reservation lookup must not even be consulted.
        assert_eq!(
            resolve_requested_alias("gguf/qwen-coder", None, || {
                panic!("a qualified name must decide without reading the registry row")
            }),
            None
        );
        // Resolved: the registry's own alias wins over the requested spelling.
        let model = registry_model("qwen-coder", None);
        assert_eq!(
            resolve_requested_alias("safetensors/qwen-coder", Some(&model), || {
                panic!("a resolved model must decide without reading the registry row")
            }),
            Some("qwen-coder".to_owned())
        );
    }

    fn registry_model(alias: &str, parser: Option<&str>) -> ModelInfo {
        ModelInfo {
            alias: alias.to_owned(),
            engine: "safetensors".to_owned(),
            vram_required_mb: 0,
            status: "available".to_owned(),
            tool_call_parser: parser.map(str::to_owned),
            source: None,
            withdrawn: false,
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
            max_generation_ms: None,
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
            max_generation_ms: None,
            tool_call_parser: None,
            extra_body: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            seed: None,
            stop: None,
            stream: None,
            stream_options: None,
            response_format: None,
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
            max_generation_ms: None,
            tool_call_parser: None,
            extra_body: None,
            max_tokens: Some(32),
            temperature: Some(0.7),
            top_p: Some(0.9),
            seed: Some(7),
            stop: Some(StopField::One("\n\n".to_owned())),
            stream: None,
            stream_options: None,
            response_format: None,
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
            max_generation_ms: None,
            tool_call_parser: None,
            extra_body: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            seed: None,
            stop: None,
            stream: None,
            stream_options: None,
            response_format: None,
        };
        let payload: serde_json::Value =
            serde_json::from_str(&build_generation_request(&request).expect("encode"))
                .expect("valid json");

        assert_eq!(payload["tools"][0]["function"]["name"], "get_weather");
        assert_eq!(payload["tool_choice"], "auto");
        assert_eq!(payload["tool_call_parser"], "qwen_coder");
    }

    #[test]
    fn a_json_object_response_format_forwards_the_permissive_object_schema() {
        // `json_object` names no schema, and forwarding nothing made it a
        // silent no-op: the client asked for JSON and got prose, with no error
        // anywhere to say the mode had been ignored. "Any object" is the whole
        // of what the mode promises, and the host's compiler expresses exactly
        // that for a schema with no declared properties.
        let format = |body: &str| {
            let request: ChatCompletionRequest = serde_json::from_str(body).expect("valid request");
            let payload: serde_json::Value =
                serde_json::from_str(&build_generation_request(&request).expect("encode"))
                    .expect("valid json");
            payload["json_schema"].clone()
        };

        assert_eq!(
            format(
                r#"{"model":"m","messages":[{"role":"user","content":"hi"}],
                    "response_format":{"type":"json_object"}}"#
            ),
            serde_json::json!(r#"{"type":"object"}"#)
        );
        // `text` is the one format that constrains nothing.
        assert_eq!(
            format(
                r#"{"model":"m","messages":[{"role":"user","content":"hi"}],
                    "response_format":{"type":"text"}}"#
            ),
            serde_json::Value::Null
        );
        // An explicit schema still wins: it is the more specific statement.
        assert_eq!(
            format(
                r#"{"model":"m","messages":[{"role":"user","content":"hi"}],
                    "response_format":{"type":"json_schema","json_schema":{"schema":{"type":"object","properties":{"a":{"type":"integer"}}}}}}"#
            ),
            serde_json::json!(r#"{"properties":{"a":{"type":"integer"}},"type":"object"}"#)
        );
    }

    /// A standard multimodal turn must survive the guest untouched.
    ///
    /// `content: Option<String>` narrowed the OpenAI message shape, and the
    /// narrowing was reachable: an array of parts failed *deserialization*, so
    /// the request never reached the host's opaque forwarding and a valid
    /// request came back as HTTP 500 — even against an upstream that reads it.
    #[test]
    fn a_multimodal_message_reaches_the_host_unchanged() {
        let body = br#"{"model":"m","messages":[{"role":"user","content":[
            {"type":"text","text":"what is this?"},
            {"type":"image_url","image_url":{"url":"https://example.invalid/a.png"}}
        ],"x-provider-hint":"vision"}]}"#;
        let request: ChatCompletionRequest =
            serde_json::from_slice(body).expect("a multimodal turn is a valid request");
        let payload: serde_json::Value =
            serde_json::from_str(&build_generation_request(&request).expect("encode"))
                .expect("valid json");

        let content = &payload["messages"][0]["content"];
        assert_eq!(content[0]["type"], "text");
        assert_eq!(
            content[1]["image_url"]["url"], "https://example.invalid/a.png",
            "the parts reach the host in the shape the client sent: {payload}"
        );
        // And a field this route does not model is carried rather than
        // dropped: it is the client's and the upstream's business.
        assert_eq!(payload["messages"][0]["x-provider-hint"], "vision");

        // The ordinary string form still round-trips as a string, not as an
        // object wrapping one.
        let plain: ChatCompletionRequest =
            serde_json::from_slice(br#"{"model":"m","messages":[{"role":"user","content":"hi"}]}"#)
                .expect("valid request");
        let payload: serde_json::Value =
            serde_json::from_str(&build_generation_request(&plain).expect("encode"))
                .expect("valid json");
        assert_eq!(payload["messages"][0]["content"], "hi");
    }

    #[test]
    fn a_json_schema_format_without_a_schema_is_refused() {
        // Dropping the constraint is the one outcome a caller cannot detect: it
        // asked for schema-conforming data, got prose, and has nothing telling
        // it the difference — so whatever it parses next treats arbitrary text
        // as structured.
        let (status, body) = route_request(
            "POST",
            ROUTE_CHAT_COMPLETIONS,
            br#"{"model":"m","messages":[{"role":"user","content":"hi"}],
                 "response_format":{"type":"json_schema","json_schema":{"name":"x"}}}"#,
        )
        .expect("a malformed format is a request error, not a host failure");
        assert_eq!(status, 400);
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("error json");
        assert_eq!(payload["error"]["type"], "invalid_request_error");
        assert!(
            payload["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("json_schema.schema")),
            "the error should name the missing field: {payload}"
        );
    }

    #[test]
    fn an_unsupported_response_format_type_is_refused() {
        // A misspelling used to fall through to unconstrained inference and a
        // 200. The caller stated a format, so answering as though it had not is
        // the same silent no-op the missing-schema refusal exists to prevent.
        let (status, body) = route_request(
            "POST",
            ROUTE_CHAT_COMPLETIONS,
            br#"{"model":"m","messages":[{"role":"user","content":"hi"}],
                 "response_format":{"type":"json-Object"}}"#,
        )
        .expect("an unsupported format is a request error, not a host failure");
        assert_eq!(status, 400);
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("error json");
        assert_eq!(payload["error"]["type"], "invalid_request_error");
        assert!(
            payload["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("json-Object")),
            "the error should quote what was sent: {payload}"
        );

        // Checked before the schema, so a misspelling is refused even when one
        // rides along and would otherwise have been honoured.
        let format = |body: &str| {
            serde_json::from_str::<ResponseFormat>(body)
                .expect("valid json")
                .schema_source()
        };
        assert!(format(r#"{"type":"bogus","json_schema":{"schema":{"type":"object"}}}"#).is_err());
        // And a recognised type still lets an explicit schema win, which is
        // what makes `json_object` with a schema precise rather than permissive.
        assert_eq!(
            format(
                r#"{"type":"json_object","json_schema":{"schema":{"type":"object","properties":{"a":{"type":"integer"}}}}}"#
            ),
            Ok(Some(
                r#"{"properties":{"a":{"type":"integer"}},"type":"object"}"#.to_owned()
            ))
        );
        assert_eq!(format(r#"{"type":"text"}"#), Ok(None));
    }

    #[test]
    fn a_stream_usage_request_reaches_the_host_envelope() {
        // Providers emit streamed usage only when the option is present, so
        // without carrying the intent the host had nothing to read and the
        // trailing usage chunk this client asked for never arrived.
        let with = |body: &str| {
            let request: ChatCompletionRequest = serde_json::from_str(body).expect("valid request");
            let payload: serde_json::Value =
                serde_json::from_str(&build_generation_request(&request).expect("encode"))
                    .expect("valid json");
            payload["include_usage"].clone()
        };

        assert_eq!(
            with(
                r#"{"model":"m","messages":[{"role":"user","content":"hi"}],
                    "stream":true,"stream_options":{"include_usage":true}}"#
            ),
            serde_json::json!(true)
        );
        // Absent unless asked: the field is one an upstream may reject, and a
        // client that never requested usage gains nothing from that risk.
        assert_eq!(
            with(r#"{"model":"m","messages":[{"role":"user","content":"hi"}],"stream":true}"#),
            serde_json::Value::Null
        );
    }

    #[test]
    fn a_call_the_request_never_offered_is_not_exposed() {
        let request =
            |tools: serde_json::Value, choice: Option<serde_json::Value>| ChatCompletionRequest {
                model: "qwen-coder".to_owned(),
                messages: vec![ChatMessage::text("user", "go")],
                tools: tools.as_array().expect("array").clone(),
                tool_choice: choice,
                max_generation_ms: None,
                tool_call_parser: None,
                extra_body: None,
                max_tokens: None,
                temperature: None,
                top_p: None,
                seed: None,
                stop: None,
                stream: None,
                stream_options: None,
                response_format: None,
            };
        let call = |name: &str| ToolCall {
            id: "call_1".to_owned(),
            kind: "function".to_owned(),
            function: ToolCallFunction {
                name: name.to_owned(),
                arguments: "{}".to_owned(),
            },
        };
        let offered = serde_json::json!([
            {"type": "function", "function": {"name": "read_file"}}
        ]);

        assert_eq!(
            request(offered.clone(), None).unoffered_call(&[call("read_file")]),
            None
        );
        // The whole point: a backend naming something else is answering a
        // question nobody put, and on an `openai:` binding that backend is a
        // third-party server the operator chose, not the client.
        assert_eq!(
            request(offered.clone(), None).unoffered_call(&[call("rm_rf")]),
            Some("rm_rf")
        );
        // A pinned `tool_choice` is an offer in its own right — a request may
        // name a call without repeating the list.
        assert_eq!(
            request(
                serde_json::json!([]),
                Some(serde_json::json!({"type":"function","function":{"name":"pinned"}}))
            )
            .unoffered_call(&[call("pinned")]),
            None
        );
        // And it is the *whole* set, not an addition to it. Unioning the pin
        // with every advertised name let a request pinned to `read_file` still
        // accept the `delete_file` call it also listed — the opposite of what
        // pinning asks for, in the more dangerous direction.
        let pinned = request(
            serde_json::json!([
                {"type": "function", "function": {"name": "read_file"}},
                {"type": "function", "function": {"name": "delete_file"}}
            ]),
            Some(serde_json::json!({"type":"function","function":{"name":"read_file"}})),
        );
        assert_eq!(pinned.unoffered_call(&[call("read_file")]), None);
        assert_eq!(
            pinned.unoffered_call(&[call("delete_file")]),
            Some("delete_file"),
            "a listed-but-not-pinned function is not what this request asked for"
        );
        // Nothing readable was offered, which is not evidence about what was in
        // the set. Refusing everything there would break a client whose tool
        // entries this side simply cannot parse a name out of.
        assert_eq!(
            request(serde_json::json!([{"type": "function"}]), None).unoffered_call(&[call("any")]),
            None
        );
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

    /// Drive a gate with fragments, returning what it streamed as content,
    /// what it held back for the buffered parser (which, once a call trips
    /// the gate or an anchored gate decides it never will, may be a *tail* of
    /// the generation rather than the whole thing — see [`StreamingContentGate::finish`]),
    /// the offset within that tail still unsent, and the full concatenated
    /// generation for tests that need the untruncated ground truth.
    fn run_gate(parser: ToolCallParser, fragments: &[&str]) -> (String, String, usize, String) {
        let mut gate = StreamingContentGate::new(parser);
        let mut streamed = String::new();
        for fragment in fragments {
            if let Some(content) = gate.push(fragment) {
                streamed.push_str(&content);
            }
        }
        let full = fragments.concat();
        let (whole, sent, overflowed) = gate.finish();
        assert!(
            !overflowed,
            "these small test fragments must never hit the retention cap"
        );
        (streamed, whole, sent, full)
    }

    #[test]
    fn streamed_content_equals_the_buffered_message_across_a_tool_call() {
        // The streaming contract is that concatenating the content deltas
        // yields the buffered message. The whitespace between prose and an
        // opener is where the two used to diverge: the buffered parser removes
        // the call region and trims, while the gate had already streamed the
        // newline.
        let (streamed, whole, sent, full) = run_gate(
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
        // `whole` may be a truncated tail of `full`: the gate drops a prefix
        // once it is both proven opener-free and already sent, so with a
        // prefix this long ("Let me check." exceeds the Qwen hold) some of it
        // never makes it into `whole` at all. Reconciling through the same
        // `unsent_tail` the production handler uses keeps the comparison in
        // one coordinate space; comparing `streamed` straight to a parse of
        // `whole` would not, since that parse's own `trim()` no longer lines
        // up with the untruncated buffered message.
        let parsed = parse_assistant_output(&request, &whole);
        assert_eq!(
            parsed.tool_calls.len(),
            1,
            "the call must still be recovered"
        );
        let tail = unsent_tail(&whole, sent, &parsed, false);
        let full_parsed = parse_assistant_output(&request, &full);
        assert_eq!(
            format!("{streamed}{tail}"),
            full_parsed.content,
            "streamed content plus the reconciled tail must equal the buffered message, whitespace included"
        );
    }

    #[test]
    fn withheld_trailing_whitespace_is_released_when_no_call_follows() {
        // The other side of the same rule: whitespace is only *deferred*, never
        // dropped. With no call, the buffered parse returns the text unchanged
        // and the caller's reconciliation emits whatever the gate still held.
        let (streamed, whole, sent, full) = run_gate(
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
        // `whole` is only the retained tail once the gate has dropped a
        // proven-safe, already-sent prefix — `full` (the raw concatenation of
        // every fragment) is the ground truth to reconstruct against.
        assert_eq!(
            format!("{streamed}{}", whole.get(sent..).unwrap_or_default()),
            full,
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
    fn a_host_supplied_call_does_not_let_the_parser_eat_the_prose() {
        // A response carrying a real structured call *and* prose that merely
        // looks like one. The parse's calls are discarded — the host's win —
        // so its opinion of the content has to be discarded with it, or the
        // regions it mistook for calls vanish from the stream while the
        // buffered reply keeps them.
        let whole = "Here is some JSON: {\"name\":\"lookup\",\"arguments\":{}}";
        let parsed = ParsedAssistantOutput {
            content: "Here is some JSON:".to_owned(),
            tool_calls: vec![ToolCall {
                id: "parsed".to_owned(),
                kind: "function".to_owned(),
                function: ToolCallFunction {
                    name: "lookup".to_owned(),
                    arguments: "{}".to_owned(),
                },
            }],
        };

        assert_eq!(
            unsent_tail(whole, 0, &parsed, true),
            whole,
            "with the host supplying the calls, the raw text is the truth"
        );

        // And when the parse *is* what supplies them, its content still wins —
        // that is the case the offset arithmetic exists for.
        assert_eq!(
            unsent_tail(whole, 0, &parsed, false),
            parsed.content,
            "a parser-recovered call still removes its own region"
        );

        // No calls anywhere: the exact byte cut, so leading whitespace the gate
        // held back is not trimmed away.
        let plain = ParsedAssistantOutput {
            content: whole.to_owned(),
            tool_calls: Vec::new(),
        };
        assert_eq!(unsent_tail(whole, 5, &plain, false), &whole[5..]);
    }

    #[test]
    fn a_legacy_function_call_survives_the_message_history() {
        // A client continuing a pre-`tool_calls` conversation replays the
        // assistant turn that made the call. With no field to land in it was
        // dropped, so the upstream saw a `role: "function"` result with no
        // request before it and answered as though it had never asked.
        let raw = serde_json::json!({
            "model": "coder",
            "messages": [
                {"role": "user", "content": "read it"},
                {"role": "assistant", "content": null,
                 "function_call": {"name": "read_file", "arguments": "{\"path\":\"a.rs\"}"}},
                {"role": "function", "name": "read_file", "content": "fn main() {}"}
            ]
        });
        let request: ChatCompletionRequest =
            serde_json::from_value(raw).expect("a legacy history is a valid request");

        let payload = build_generation_request(&request).expect("request builds");
        let payload: serde_json::Value = serde_json::from_str(&payload).expect("valid JSON");
        let assistant = &payload["messages"][1];
        assert_eq!(
            assistant["function_call"]["name"], "read_file",
            "the call the assistant made has to reach the host: {payload}"
        );
        // And the result stays paired with it, which is what makes the pair
        // legible to a provider that still speaks this dialect.
        assert_eq!(payload["messages"][2]["name"], "read_file");
        assert_eq!(payload["messages"][2]["content"], "fn main() {}");
    }

    #[test]
    fn a_requested_generation_budget_reaches_the_host() {
        // Not an OpenAI field, so it used to be dropped: the host fell back to
        // its own default and a client asking for a shorter leash waited out
        // the long one. Accepted top level and via `extra_body`, because
        // clients differ on where they keep nonstandard options.
        let mut request = tool_request_named("qwen3-coder");
        request.max_generation_ms = Some(5_000);
        let payload = build_generation_request(&request).expect("request builds");
        let payload: serde_json::Value = serde_json::from_str(&payload).expect("valid JSON");
        assert_eq!(payload["max_generation_ms"], 5_000);

        let mut via_extra = tool_request_named("qwen3-coder");
        via_extra.extra_body = Some(ExtraBody {
            tool_call_parser: None,
            max_generation_ms: Some(7_000),
        });
        let payload = build_generation_request(&via_extra).expect("request builds");
        let payload: serde_json::Value = serde_json::from_str(&payload).expect("valid JSON");
        assert_eq!(payload["max_generation_ms"], 7_000);

        // Omission stays omission, so the binding's own default still applies.
        let payload =
            build_generation_request(&tool_request_named("qwen3-coder")).expect("request builds");
        let payload: serde_json::Value = serde_json::from_str(&payload).expect("valid JSON");
        assert!(payload.get("max_generation_ms").is_none());
    }

    #[test]
    fn a_configured_endpoint_that_is_not_found_is_the_gateway_s_failure() {
        // The upstream model name and the endpoint path both come from the
        // binding. A client picks the mesh alias and nothing else, so
        // `invalid_request_error` sends it hunting for a mistake in a request
        // it cannot change while the real fault goes unreported.
        let not_found = bindings::tachyon::accelerator::cpu::GenerationError {
            message: "upstream said 404".to_owned(),
            upstream_status: Some(404),
            invalid_request: false,
        };
        let (status, _body) = generation_error_payload("coder", not_found);
        assert_eq!(status, 502);

        // A 400 really is about the forwarded parameters and stays the
        // caller's.
        let rejected = bindings::tachyon::accelerator::cpu::GenerationError {
            message: "upstream said 400".to_owned(),
            upstream_status: Some(400),
            invalid_request: false,
        };
        let (status, _body) = generation_error_payload("coder", rejected);
        assert_eq!(status, 400);
    }

    #[test]
    fn tool_choice_none_withdraws_the_offer_the_tools_list_makes() {
        // The schema's way of saying "you may see the tools, do not call them",
        // and a nonempty list alongside it is how that is normally sent — so
        // reading intent from the list alone armed the parser for exactly the
        // request that asked it not to.
        let mut request = tool_request_named("qwen3-coder");
        request.tool_choice = Some(serde_json::json!("none"));
        assert!(
            !request.has_tool_intent(),
            "`tool_choice: none` is an offer withdrawn, whatever the tools list says"
        );
        assert!(
            request.resolved_tool_call_parser().is_none(),
            "no parser may recover calls from prose for a request that ruled them out"
        );

        // A named function is the opposite instruction and must still arm it.
        request.tool_choice = Some(serde_json::json!({
            "type": "function",
            "function": {"name": "read_file"}
        }));
        assert!(
            request.has_tool_intent(),
            "naming a function is a request for a call, not a withdrawal"
        );
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
        // Reported by the provider with nothing behind it. `has_tool_calls` is
        // read from the same list that fills `message.tool_calls`, so `false`
        // here means the client would be sent to an empty array and told to
        // dispatch from it — an agent then waits on a call nobody made.
        assert_eq!(resolve_finish_reason(Some("tool_calls"), false), "stop");
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
            invalid_request: false,
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
                invalid_request: false,
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
                invalid_request: false,
            },
        );
        assert_eq!(status, 502);

        // A local *host* failure has no remote status to relay and stays a 500.
        let (status, _) = generation_error_payload(
            "coder",
            bindings::tachyon::accelerator::cpu::GenerationError {
                message: "model alias `coder` is not loaded".to_owned(),
                upstream_status: None,
                invalid_request: false,
            },
        );
        assert_eq!(status, 500);

        // A local rejection of the caller's own parameters has no status
        // either, and must not be reported as one: a 500 tells the client to
        // retry a request that cannot succeed however many times it is sent.
        let (status, body) = generation_error_payload(
            "coder",
            bindings::tachyon::accelerator::cpu::GenerationError {
                message: "max_new_tokens 8192 exceeds the binding ceiling".to_owned(),
                upstream_status: None,
                invalid_request: true,
            },
        );
        assert_eq!(status, 400);
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("error body");
        assert_eq!(payload["error"]["type"], "invalid_request_error");
    }

    #[test]
    fn streaming_gate_forwards_prose_and_withholds_a_tagged_tool_call() {
        // The prose before the call must reach the client as it is generated —
        // buffering everything would cost time-to-first-token on every request
        // that merely offers tools.
        let (streamed, whole, sent, _full) = run_gate(
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
        // And `sent` stops exactly where the withheld newline starts, in the
        // gate's own (possibly truncated) coordinate space — not past it. The
        // handler decides that newline's fate once it has parsed the whole
        // text: internal whitespace if prose follows the call, trimmed if not.
        assert!(
            whole[sent..].starts_with('\n'),
            "the withheld separator must still be there, unclaimed by `sent`"
        );
        // The tag itself never leaks into the content stream.
        assert!(!streamed.contains("<tool_call>"));
        assert!(whole.contains("<tool_call>"));
    }

    #[test]
    fn a_false_positive_json_gate_still_delivers_its_leading_whitespace() {
        // An anchored gate trips at the first `{` even when the answer turns
        // out to be ordinary JSON prose rather than a tool call. Nothing is
        // streamed at that point, so `sent` must stay 0: it once advanced to
        // the opener, and the handler then cut the tail there and dropped the
        // leading whitespace the buffered response keeps.
        let (streamed, whole, sent, _full) =
            run_gate(ToolCallParser::Json, &["  {\"answer\"", ":1}"]);
        assert!(streamed.is_empty(), "an anchored gate streams nothing");
        assert_eq!(sent, 0, "nothing was sent, so nothing may be skipped");

        // What the handler would emit as the tail, for a parse that found no
        // calls: the whole text, whitespace included.
        let mut request = tool_request_named("local");
        request.tool_call_parser = Some(ToolCallParser::Json);
        let parsed = parse_assistant_output(&request, &whole);
        assert!(
            parsed.tool_calls.is_empty(),
            "`{{\"answer\":1}}` is prose, not a call"
        );
        let tail = whole.get(sent..).unwrap_or_default();
        assert_eq!(
            format!("{streamed}{tail}"),
            parsed.content,
            "streamed content must equal the buffered message, leading whitespace included"
        );
    }

    #[test]
    fn a_separator_before_a_call_is_restored_when_prose_follows_it() {
        // The gate withholds the whitespace before an opener because the
        // buffered parser trims it — but only when the call ends the message.
        // With prose after the call that whitespace is *internal*, and the
        // buffered parser keeps it, so the tail has to hand it back.
        let (streamed, whole, sent, _full) = run_gate(
            ToolCallParser::Qwen,
            &[
                "Hi \n",
                "<tool_call>",
                r#"{"name":"search","arguments":{}}"#,
                "</tool_call>",
                "AFTER",
            ],
        );
        assert_eq!(streamed, "Hi", "the separator is withheld, not sent");
        assert_eq!(sent, 2, "`sent` counts what went out, not what was skipped");

        let mut request = tool_request_named("local");
        request.tool_call_parser = Some(ToolCallParser::Qwen);
        let parsed = parse_assistant_output(&request, &whole);
        assert_eq!(parsed.tool_calls.len(), 1, "the call is still recovered");
        let consumed = whole.get(..sent).unwrap_or_default().trim_start().len();
        let tail = parsed.content.get(consumed..).unwrap_or_default();
        assert_eq!(
            format!("{streamed}{tail}"),
            parsed.content,
            "`Hi \\nAFTER` buffered must not stream as `HiAFTER`"
        );
    }

    #[test]
    fn streaming_gate_matches_an_opener_split_across_fragments() {
        // `<tool_` / `call>` arriving separately must still be caught, or the
        // opening tag leaks into the transcript.
        let (streamed, _, _, _full) = run_gate(
            ToolCallParser::Qwen,
            &["hi ", "<tool_", "call>{\"name\":\"f\"}</tool_call>"],
        );
        // The space before the opener is trimmed by the buffered parser, so it
        // is not streamed either.
        assert_eq!(streamed, "hi");
    }

    #[test]
    fn streaming_gate_streams_everything_when_no_tool_call_appears() {
        let (streamed, whole, sent, full) =
            run_gate(ToolCallParser::Qwen, &["all ", "plain ", "prose"]);
        // The tail is held back until the stream ends; the handler flushes it
        // from the parsed content afterwards. `whole` may only be a retained
        // tail once a proven-safe, already-sent prefix has been dropped, so
        // reconstruction is checked against `full` — the raw concatenation of
        // every fragment — rather than `whole` itself.
        assert_eq!(
            format!("{streamed}{}", whole.get(sent..).unwrap_or_default()),
            full,
        );
    }

    #[test]
    fn streaming_gate_withholds_an_anchored_json_call_entirely() {
        // A `json` response is a tool call only when the *whole* output is one
        // JSON value, so nothing may be streamed as content.
        let (streamed, whole, sent, _full) = run_gate(
            ToolCallParser::Json,
            &["{\"tool_calls\":[{\"name\":", "\"search\"}]}"],
        );
        assert!(streamed.is_empty());
        assert_eq!(sent, 0);
        assert!(whole.starts_with('{'));
    }

    #[test]
    fn streaming_gate_does_not_anchor_on_a_brace_inside_prose() {
        // A `{` mid-sentence cannot start a JSON tool call — the whole output
        // would have to parse — so it must not stop content from streaming.
        let (_, _, _, full) = run_gate(ToolCallParser::Json, &["use ", "Vec<T> { .. } ", "here"]);
        let mut gate = StreamingContentGate::new(ToolCallParser::Json);
        gate.push(&full);
        assert!(!gate.tripped, "a brace inside prose must not trip the gate");
    }

    #[test]
    fn streaming_gate_withholds_the_mistral_marker() {
        let (streamed, _, _, _full) = run_gate(
            ToolCallParser::Mistral,
            &["Checking\n", "[TOOL_CALLS] [{\"name\":\"fetch\"}]"],
        );
        assert_eq!(streamed, "Checking");
    }

    #[test]
    fn an_unanchored_gate_retains_o_hold_not_o_n_for_a_long_call_free_answer() {
        // The primary acceptance bar: a tool-enabled stream of N bytes with no
        // call retains O(hold), not O(N). Feed a gate far more prose than the
        // Qwen hold (12 bytes) and check what it is holding onto at the end,
        // not just what it streamed.
        let mut gate = StreamingContentGate::new(ToolCallParser::Qwen);
        let mut streamed = String::new();
        let line = "no call ever appears in this sentence, only prose. ";
        let mut full = String::new();
        for _ in 0..500 {
            full.push_str(line);
            if let Some(content) = gate.push(line) {
                streamed.push_str(&content);
            }
        }
        // A drop can lag by up to one extra fragment when it lands right
        // before a still-unresolved separator (see the note in `push`), so
        // the bound allows a small multiple of a single line rather than
        // asserting the tightest possible byte count — the point being
        // tested is boundedness relative to the 26 KB answer, not an exact
        // constant.
        assert!(
            gate.seen.len() <= gate.hold + 4 * line.len(),
            "a call-free stream must not retain what it has already proven safe and sent, got {} bytes retained out of {} generated",
            gate.seen.len(),
            full.len(),
        );
        let (whole, sent, overflowed) = gate.finish();
        assert!(!overflowed);
        assert_eq!(
            format!("{streamed}{}", whole.get(sent..).unwrap_or_default()),
            full,
            "bounding retention must not lose or duplicate any byte of the answer"
        );
    }

    #[test]
    fn a_single_huge_call_free_fragment_does_not_trip_the_retention_cap() {
        // The backend is free to hand the whole generation to `push` as one
        // fragment (SSE frames are accepted up to 1 MiB), and that fragment
        // can easily be call-free prose bigger than `MAX_GATED_RETENTION` —
        // a large code explanation, say. Absorbing it in one shot before
        // ever running the opener/safe-release logic would trip the cap on
        // a response that has nothing wrong with it; `push` has to walk it
        // in slices instead so retention stays bounded exactly as it would
        // if the same bytes had arrived as many small fragments.
        let line = "no call ever appears in this sentence, only prose. ";
        let full = line.repeat(20_000); // well past MAX_GATED_RETENTION (256 KiB)
        assert!(full.len() > MAX_GATED_RETENTION);

        let mut gate = StreamingContentGate::new(ToolCallParser::Qwen);
        let streamed = gate.push(&full).unwrap_or_default();
        assert!(
            gate.seen.len() <= gate.hold + PUSH_CHUNK_SIZE,
            "a call-free fragment must not retain anywhere near its own size, got {} bytes retained out of {} pushed",
            gate.seen.len(),
            full.len(),
        );
        let (whole, sent, overflowed) = gate.finish();
        assert!(
            !overflowed,
            "a legitimate call-free fragment must not be treated as a retention-cap failure"
        );
        assert_eq!(
            format!("{streamed}{}", whole.get(sent..).unwrap_or_default()),
            full,
            "slicing the fragment internally must not lose or duplicate any byte"
        );
    }

    #[test]
    fn a_pin_gets_room_to_resolve_before_the_cap_gives_up_on_it() {
        // A stray `<` followed closely enough by more `<`s to keep extending
        // the pin (dense nested generics are a realistic source) can stay
        // unresolved for a while without ever containing a call. Capping the
        // fragment before the wall search ever gets to look at it would
        // starve it of exactly the bytes that would let it resolve — even
        // though, once it does, retention collapses straight back down.
        let mut gate = StreamingContentGate::new(ToolCallParser::Qwen);
        // Chained false starts, each well within `hold` (12 bytes) of the
        // next, past MAX_GATED_RETENTION — the pin never gets to clear on
        // its own here.
        let chain = "<a".repeat(140_000);
        assert!(chain.len() > MAX_GATED_RETENTION);
        gate.push(&chain);
        assert!(
            gate.drop_ceiling.is_some(),
            "the chain must still be unresolved after nothing but chained false starts"
        );
        // Ordinary prose, long enough to provide a wall well within the
        // extra headroom `absorb_capped` grants a still-resolving pin.
        let tail = "ordinary prose with no angle brackets at all. ".repeat(50);
        gate.push(&tail);
        assert!(
            gate.drop_ceiling.is_none(),
            "the wall in the trailing prose must have been given the chance to resolve the pin"
        );
        let (whole, _sent, overflowed) = gate.finish();
        assert!(
            !overflowed,
            "a pin that successfully resolves must not be reported as an overflow"
        );
        let mut request = tool_request_named("local");
        request.tool_call_parser = Some(ToolCallParser::Qwen);
        let parsed = parse_assistant_output(&request, &whole);
        assert!(parsed.tool_calls.is_empty());
    }

    #[test]
    fn a_wall_beyond_the_safe_prefix_does_not_panic_the_resumed_search() {
        // The wall search can resolve a pin using bytes well past what this
        // round's `drop_amount` covers (it looks as far ahead as the chain
        // needs, independent of how much is currently safe to stream). When
        // that happens, searching for a fresh pin afterward must not slice
        // `[search_ceiling_from..drop_amount)` with a start past the end.
        let mut gate = StreamingContentGate::new(ToolCallParser::Qwen);
        for ch in "</tool_calls>{".chars() {
            let mut buf = [0u8; 4];
            gate.push(ch.encode_utf8(&mut buf));
        }
        let (_, _, overflowed) = gate.finish();
        assert!(!overflowed);
    }

    #[test]
    fn a_decided_anchored_gate_stops_accumulating_entirely() {
        // Once an anchored gate's leading bytes diverge from every opener, no
        // later byte can retroactively open a call, so nothing more is worth
        // buffering — the cheap win the JSON parser gets from anchoring.
        let mut gate = StreamingContentGate::new(ToolCallParser::Json);
        assert_eq!(
            gate.push("plainly not a call, "),
            Some("plainly not a call, ".to_owned())
        );
        assert!(gate.decided_no_call);
        assert!(
            gate.seen.is_empty(),
            "a decided gate must not retain anything it no longer needs to re-examine"
        );
        // Further fragments — even ones that would otherwise look like an
        // opener — are forwarded untouched rather than buffered.
        assert_eq!(
            gate.push("{ this looks like json but isn't at the anchor }"),
            Some("{ this looks like json but isn't at the anchor }".to_owned())
        );
        assert!(gate.seen.is_empty());
    }

    #[test]
    fn a_tripped_gate_never_retains_more_than_the_cap() {
        // A hostile upstream that opens a call and never closes it must not be
        // able to pin unbounded memory in the gate.
        let mut gate = StreamingContentGate::new(ToolCallParser::Qwen);
        gate.push("<tool_call>");
        assert!(gate.tripped);
        let chunk = "x".repeat(64 * 1024);
        for _ in 0..8 {
            gate.push(&chunk);
        }
        assert!(
            gate.seen.len() <= MAX_GATED_RETENTION,
            "a tripped gate must be capped, got {} bytes retained",
            gate.seen.len(),
        );
        // A capped gate is not a shorter-but-still-correct call: the buffered
        // parser can no longer find the closing tag for what was truncated,
        // so the caller must be told the retained text is not trustworthy
        // rather than silently handed a corrupted completion.
        assert!(
            gate.overflowed,
            "discarding bytes to stay under the cap must be flagged, not silent"
        );
    }

    #[test]
    fn a_separator_longer_than_hold_stays_internal_when_prose_follows_the_call() {
        // Dropping a prefix always keeps a one-byte anchor: without it, a
        // drop that lands right at the start of a still-unresolved separator
        // would make that separator the new start of the retained text, and
        // the buffered parser's `trim()` cannot tell that apart from the true
        // start of the message — it would eat a separator that has to stay
        // internal once prose follows the call. Both the leading prose and
        // the separator here exceed the Qwen hold (12 bytes) so a naive drop
        // would have already discarded the prose by the time the separator is
        // fully seen.
        let prose = "A".repeat(40);
        let separator = " ".repeat(40);
        let (streamed, whole, sent, full) = run_gate(
            ToolCallParser::Qwen,
            &[
                &prose,
                &separator,
                "<tool_call>",
                r#"{"name":"search","arguments":{}}"#,
                "</tool_call>",
                "AFTER",
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
        let tail = unsent_tail(&whole, sent, &parsed, false);
        let full_parsed = parse_assistant_output(&request, &full);
        assert_eq!(
            format!("{streamed}{tail}"),
            full_parsed.content,
            "the separator between the prose and `AFTER` must not be lost, whatever the gate dropped along the way"
        );
    }

    #[test]
    fn a_drop_boundary_inside_a_multibyte_character_does_not_panic() {
        // `safe` is always a char boundary (it comes out of `.trim_end().len()`
        // on a valid `&str`), but `safe - 1` is not necessarily one when the
        // last character of the safe prefix is multibyte. `é` is 2 bytes, so
        // a run of them followed by plain ASCII puts exactly this shape in
        // front of the gate.
        let prose = "é".repeat(20) + &"b".repeat(20);
        let mut gate = StreamingContentGate::new(ToolCallParser::Qwen);
        let _ = gate.push(&prose);
        let (_whole, _sent, overflowed) = gate.finish();
        assert!(!overflowed);
    }

    #[test]
    fn a_malformed_nested_tag_does_not_fabricate_a_call_via_a_dropped_prefix() {
        // `parse_tagged_tool_calls` mutates as it goes: removing one matched
        // pair can splice previously non-adjacent bytes together into a tag
        // that was never literally in the response. `<tool_calls` (missing
        // its `>`) directly followed by a well-formed `<tool_call>bad</tool_call>`
        // is exactly that: removing the inner match joins the leftover
        // `<tool_calls` to the `>` right after it, completing an opener that
        // then swallows the second, genuinely well-formed `<tool_calls>...`
        // tag as part of its own (invalid-JSON) payload — so the buffered
        // parse of the *full* text finds no calls at all.
        //
        // If the gate had already dropped `<tool_calls` from `seen` (proven,
        // wrongly, "safe" because it doesn't itself complete into a literal
        // opener), the buffered parse of the truncated tail never sees that
        // merge: the second `<tool_calls>{"name":"y"}</tool_calls>` is found
        // on its own, with valid JSON, and fabricates a dispatchable call.
        let raw = r#"<tool_calls<tool_call>bad</tool_call>><tool_calls>{"name":"y"}</tool_calls>"#;
        let mut request = tool_request_named("local");
        request.tool_call_parser = Some(ToolCallParser::Qwen);
        let buffered = parse_assistant_output(&request, raw);
        assert!(
            buffered.tool_calls.is_empty(),
            "sanity check on the buffered path: the malformed nesting must not parse as a call"
        );

        // Push it one byte at a time — the tightest fragmentation, and the
        // one that gives the per-fragment drop the most chances to fire
        // before the whole picture is in.
        let mut gate = StreamingContentGate::new(ToolCallParser::Qwen);
        let mut streamed = String::new();
        for ch in raw.chars() {
            let mut buf = [0u8; 4];
            if let Some(c) = gate.push(ch.encode_utf8(&mut buf)) {
                streamed.push_str(&c);
            }
        }
        let (whole, sent, overflowed) = gate.finish();
        assert!(!overflowed);
        let parsed = parse_assistant_output(&request, &whole);
        let tail = unsent_tail(&whole, sent, &parsed, false);
        assert!(
            parsed.tool_calls.is_empty(),
            "the streaming path must not fabricate a call the buffered path never found; got {:?} from whole={whole:?}",
            parsed.tool_calls,
        );
        assert_eq!(
            format!("{streamed}{tail}"),
            buffered.content,
            "reconstructed content must match the buffered message"
        );
    }

    #[test]
    fn a_rewrite_sensitive_prefix_is_withheld_from_streaming_too() {
        // Not just what gets dropped from `seen` — what gets *streamed* to
        // the client is just as exposed to the same rewrite risk, and once a
        // byte has gone out over SSE there is no taking it back regardless of
        // what `finish()` later reconstructs internally.
        //
        // `<tool_calls` (missing its `>`) directly followed by a well-formed
        // `<tool_call>{"name":"x"}</tool_call>` is a real, valid call on the
        // buffered path — but the leftover `<tool_calls` rejoins the `>`
        // right after the removed inner match into a `<tool_calls>` that
        // then swallows the second, genuinely well-formed
        // `<tool_calls>{"name":"y"}</tool_calls>` tag as its own payload, so
        // the buffered content ends up fully empty (the whole text is
        // consumed by the one real call plus the two merges). Before the
        // gate knew any of that, `<tool_calls` looked like ordinary,
        // trickle-released "safe" content — exactly what must never reach
        // the client if the buffered answer never contained it.
        let raw = r#"<tool_calls<tool_call>{"name":"x"}</tool_call>><tool_calls>{"name":"y"}</tool_calls>"#;
        let mut request = tool_request_named("local");
        request.tool_call_parser = Some(ToolCallParser::Qwen);
        let buffered = parse_assistant_output(&request, raw);
        assert_eq!(
            buffered.tool_calls.len(),
            1,
            "sanity check on the buffered path: exactly the first, well-formed call survives"
        );
        assert_eq!(
            buffered.content, "",
            "sanity check on the buffered path: nothing is left over as prose"
        );

        let mut gate = StreamingContentGate::new(ToolCallParser::Qwen);
        let mut streamed = String::new();
        for ch in raw.chars() {
            let mut buf = [0u8; 4];
            if let Some(c) = gate.push(ch.encode_utf8(&mut buf)) {
                streamed.push_str(&c);
            }
        }
        assert_eq!(
            streamed, "",
            "the malformed prefix must be withheld, not trickled out as content before it is known safe"
        );
        let (whole, sent, overflowed) = gate.finish();
        assert!(!overflowed);
        let parsed = parse_assistant_output(&request, &whole);
        assert_eq!(parsed.tool_calls.len(), 1);
        let tail = unsent_tail(&whole, sent, &parsed, false);
        assert_eq!(
            format!("{streamed}{tail}"),
            buffered.content,
            "reconstructed content must match the buffered message"
        );
    }

    #[test]
    fn mistral_never_pins_the_drop_ceiling_on_an_incidental_bracket() {
        // Mistral's parser does one split on a single marker and never
        // rescans or mutates, so a dropped prefix has nothing to rejoin
        // with — `drop_ceiling` exists for the tagged dialects' rewriting
        // parser, not this one. Ordinary code containing `[` (array
        // indexing, literals) must not fall back to O(N) retention just
        // because it looks tag-adjacent to the wrong parser.
        let mut gate = StreamingContentGate::new(ToolCallParser::Mistral);
        let line = "result = items[0] + items[1]; // ordinary code, no call here. ";
        let mut full = String::new();
        for _ in 0..5_000 {
            full.push_str(line);
            gate.push(line);
        }
        assert!(full.len() > MAX_GATED_RETENTION);
        assert!(
            gate.drop_ceiling.is_none(),
            "Mistral must never engage the tag-rewrite protection at all"
        );
        assert!(
            gate.seen.len() <= gate.hold + 4 * line.len(),
            "an incidental `[` must not stop a call-free Mistral stream from staying bounded, got {} bytes retained",
            gate.seen.len(),
        );
        let (_, _, overflowed) = gate.finish();
        assert!(!overflowed);
    }

    #[test]
    fn a_qwen_drop_ceiling_clears_once_a_wall_proves_the_chain_is_broken() {
        // The pin `drop_ceiling` puts on a false tag start is not meant to be
        // permanent: an ordinary `<` in code (`Vec<T>`) that is followed by
        // plenty of unrelated content afterward can never be spliced back
        // together with anything later, because that unrelated content can
        // never itself be part of a removed tag region. Once such a byte is
        // seen the pin must clear, or a single incidental `<` early in a
        // long, call-free response would push it toward the retention cap.
        let mut gate = StreamingContentGate::new(ToolCallParser::Qwen);
        gate.push("fn f() -> Vec<T> { "); // a `<` that never becomes a tag
                                          // The `<` starts out inside the trailing `hold` bytes, so it takes a
                                          // little more content arriving after it before the gate's own
                                          // opener-length lookahead resolves it as "definitely not a tag" and
                                          // the drop logic notices it at all.
        gate.push("return Default::default();");
        assert!(
            gate.drop_ceiling.is_some(),
            "the `<` must be pinned once it is resolved as unmatched but not yet walled off"
        );
        let line = "ordinary prose with no angle brackets in it at all. ";
        let mut full = String::from("fn f() -> Vec<T> { return Default::default();");
        for _ in 0..6_000 {
            full.push_str(line);
            gate.push(line);
        }
        assert!(full.len() > MAX_GATED_RETENTION);
        assert!(
            gate.drop_ceiling.is_none(),
            "a wall of ordinary content after the `<` must clear the pin"
        );
        assert!(
            gate.seen.len() <= gate.hold + 4 * line.len(),
            "once cleared, retention must go back to bounded, got {} bytes retained",
            gate.seen.len(),
        );
        let (whole, _sent, overflowed) = gate.finish();
        assert!(!overflowed);
        let mut request = tool_request_named("local");
        request.tool_call_parser = Some(ToolCallParser::Qwen);
        let parsed = parse_assistant_output(&request, &whole);
        assert!(parsed.tool_calls.is_empty());
    }

    #[test]
    fn prose_directly_against_a_split_opener_keeps_the_gap_after_the_call() {
        // The same failure mode without any separator at all: prose runs
        // straight into the opener (long enough, and split awkwardly enough,
        // that the whole prose prefix would be dropped before the trip), and
        // the call is followed directly by a space and more prose. With no
        // anchor byte, the retained text would begin exactly at the opener,
        // and after the tag is excised the space before `AFTER` would look
        // like leading whitespace to the buffered parser and get trimmed away.
        let prefix = "P".repeat(40);
        let (streamed, whole, sent, full) = run_gate(
            ToolCallParser::Qwen,
            &[
                &prefix,
                "<tool_calls",
                r#">{"name":"search","arguments":{}}</tool_calls>"#,
                " AFTER",
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
        let tail = unsent_tail(&whole, sent, &parsed, false);
        let full_parsed = parse_assistant_output(&request, &full);
        assert_eq!(
            format!("{streamed}{tail}"),
            full_parsed.content,
            "the space between the closing tag and `AFTER` must not be lost"
        );
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
            max_generation_ms: None,
            tool_call_parser: None,
            extra_body: None,
            max_tokens: None,
            temperature: None,
            top_p: None,
            seed: None,
            stop: None,
            stream: None,
            stream_options: None,
            response_format: None,
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
