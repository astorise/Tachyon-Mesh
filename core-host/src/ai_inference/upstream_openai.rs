//! Passthrough backend for an OpenAI-compatible upstream inference server.
//!
//! Tachyon's native Candle runtime only executes the checkpoint formats and
//! architectures it has verified loaders for. This backend covers the rest of
//! the ecosystem — llama.cpp's `llama-server`, vLLM, SGLang, or any other
//! server speaking the OpenAI chat-completions wire format — by forwarding the
//! host's own generation request to it and returning the generated text
//! unchanged. The mesh keeps ownership of routing, QoS, authorisation, and the
//! `/ai/v1` surface; only the tensor math moves out of process.
//!
//! A binding opts in through its `path`, exactly like the `mock:` scheme:
//!
//! ```text
//! openai:http://127.0.0.1:8080/v1
//! openai:https://gpu-node.lan:8000/v1?model=qwen3-coder-30b&timeout_ms=180000&max_new_tokens=4096
//! ```
//!
//! The upstream model name defaults to the binding alias and can be overridden
//! with the `model` query parameter, so a mesh alias never has to match the
//! name the upstream server happens to use. `timeout_ms` bounds each request
//! and `max_new_tokens` sets this binding's generation budget.
//!
//! Credentials are never written in the binding. The backend reads a bearer
//! token from `TACHYON_UPSTREAM_API_KEY_<ALIAS>` (alias upper-cased, every
//! non-alphanumeric byte folded to `_`), falling back to
//! `TACHYON_UPSTREAM_API_KEY`. Absent both, the request is sent unauthenticated
//! — the common case for a llama.cpp server on a trusted mesh link.

use std::{env, io::BufRead, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use thiserror::Error;

use super::candle_llm_runtime::MAX_PROMPT_BYTES_CEILING;

/// Binding `path` prefix that selects this backend.
pub(crate) const UPSTREAM_SCHEME: &str = "openai:";
/// Per-alias bearer token variable prefix.
const API_KEY_ENV_PREFIX: &str = "TACHYON_UPSTREAM_API_KEY_";
/// Fallback bearer token variable, shared by every upstream binding.
const API_KEY_ENV_FALLBACK: &str = "TACHYON_UPSTREAM_API_KEY";
/// Default per-request budget. Generous on purpose: an agentic coding request
/// against a 30B model on a busy queue routinely runs into the minutes, and a
/// timeout here surfaces as a failed request, not a retry.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);
/// Hard ceiling on a configured `timeout_ms`, so a typo cannot wedge a
/// scheduler slot indefinitely.
const MAX_TIMEOUT: Duration = Duration::from_secs(3600);
/// Cap on a single upstream response body. Matches the intent of the native
/// runtime's `max_prompt_bytes`: a runaway upstream must not exhaust host RAM.
const MAX_RESPONSE_BYTES: u64 = 64 * 1024 * 1024;
/// Cap on the error excerpt echoed back to the operator. Read as a byte limit
/// so the allocation is bounded, not just the resulting string.
const MAX_ERROR_BODY_BYTES: u64 = 2048;
/// Cap on a whole SSE stream. Generous — 64 MiB of text is millions of tokens —
/// but finite, so a stream that never terminates cannot grow without bound.
const MAX_STREAM_BYTES: u64 = MAX_RESPONSE_BYTES;
/// Cap on one SSE frame. `BufRead::read_line` grows its buffer until a newline,
/// so a single unterminated line is the tighter of the two risks.
const MAX_SSE_FRAME_BYTES: usize = 1024 * 1024;
/// Hard ceiling on `max_new_tokens` for an upstream request.
///
/// Deliberately far above the native runtime's `HOST_MAX_NEW_TOKENS`, because
/// the two limits protect different things. The native cap bounds *this host's*
/// decode loop, where every extra token costs a forward pass and grows a KV
/// cache in local VRAM. An upstream generation costs this node one open HTTP
/// connection: the resources it consumes belong to the remote server, which
/// enforces its own context window and queueing.
///
/// So the cap here exists only to keep a request finite — the real bound is
/// `timeout_ms` — and is set high enough for the agentic coding workloads this
/// backend exists to serve, where 256 tokens truncates mid-function.
pub(crate) const UPSTREAM_MAX_NEW_TOKENS: usize = 8192;
/// Applied when a request omits `max_new_tokens`, so the upstream's own default
/// (possibly unlimited) never governs the budget.
pub(crate) const UPSTREAM_DEFAULT_MAX_NEW_TOKENS: usize = 2048;

#[derive(Debug, Error)]
pub(crate) enum UpstreamError {
    #[error("upstream binding `{alias}` is invalid: {detail}")]
    InvalidBinding { alias: String, detail: String },
    #[error("upstream request for model `{alias}` is invalid: {detail}")]
    InvalidRequest { alias: String, detail: String },
    #[error("upstream `{alias}` at `{endpoint}` failed: {detail}")]
    Transport {
        alias: String,
        endpoint: String,
        detail: String,
    },
    #[error("upstream `{alias}` at `{endpoint}` returned HTTP {status}: {body}")]
    Status {
        alias: String,
        endpoint: String,
        status: u16,
        body: String,
    },
    #[error("upstream `{alias}` returned an unusable response: {detail}")]
    MalformedResponse { alias: String, detail: String },
}

/// A parsed `openai:` binding. Split out from the runtime so binding validation
/// is testable without constructing an HTTP client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UpstreamEndpoint {
    /// Base URL with any trailing `/` removed, e.g. `http://127.0.0.1:8080/v1`.
    pub(crate) base_url: String,
    /// Model name sent upstream. Defaults to the binding alias.
    pub(crate) model: String,
    pub(crate) timeout: Duration,
    /// Budget applied when a request omits `max_new_tokens`, and the ceiling
    /// every request is validated against. Per-binding so one alias can serve
    /// long agentic completions while another stays short, without moving a
    /// global constant.
    pub(crate) max_new_tokens: usize,
}

impl UpstreamEndpoint {
    /// Parse `openai:<url>[?model=…][&timeout_ms=…]`.
    ///
    /// Returns `Ok(None)` when the path does not use the `openai:` scheme, so
    /// the caller can fall through to the on-disk loaders; `Err` only when the
    /// scheme *is* claimed but the rest is unusable.
    pub(crate) fn parse(alias: &str, path: &str) -> Result<Option<Self>, UpstreamError> {
        let Some(remainder) = path.trim().strip_prefix(UPSTREAM_SCHEME) else {
            return Ok(None);
        };
        let invalid = |detail: String| UpstreamError::InvalidBinding {
            alias: alias.to_owned(),
            detail,
        };

        let (url, query) = match remainder.split_once('?') {
            Some((url, query)) => (url, Some(query)),
            None => (remainder, None),
        };
        let url = url.trim_end_matches('/');
        // Parse structurally rather than matching on the prefix: a textual
        // check accepts authority-less forms like `http:///v1`, which reqwest
        // only rejects when the first request is sent — far too late for a
        // load-time validation contract.
        //
        // A structural failure and an empty authority share one message: both
        // mean "this is not an absolute http(s) URL with a host", and the
        // url crate's own wording ("relative URL without a base", "empty host")
        // does not tell an operator what to write instead.
        let malformed = |detail: &str| {
            invalid(format!(
                "`{url}` is not usable as an upstream URL ({detail}); expected `{UPSTREAM_SCHEME}http://…` or `{UPSTREAM_SCHEME}https://…` with a host, e.g. `{UPSTREAM_SCHEME}http://127.0.0.1:8080/v1`"
            ))
        };
        let parsed = reqwest::Url::parse(url).map_err(|error| malformed(&error.to_string()))?;
        // Userinfo would be turned into a Basic `Authorization` header by
        // reqwest, silently bypassing the environment-only credential contract
        // — and `base_url` is echoed in telemetry and transport errors, so the
        // secret would leak there too.
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(invalid(
                "upstream URL must not embed credentials; set the bearer token in `TACHYON_UPSTREAM_API_KEY_<ALIAS>` or `TACHYON_UPSTREAM_API_KEY`".to_owned(),
            ));
        }
        // A fragment is never sent on the wire, and `base_url` is concatenated
        // with the route suffix — `http://h/v1#x` + `/chat/completions` reaches
        // the upstream as bare `/v1`, so the binding would load and then always
        // hit the wrong path.
        if parsed.fragment().is_some() {
            return Err(invalid(format!(
                "upstream URL `{url}` must not contain a `#` fragment: it is never sent on the wire, so request paths would silently lose their suffix"
            )));
        }
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(malformed(&format!("scheme `{}`", parsed.scheme())));
        }
        if parsed.host_str().is_none_or(str::is_empty) {
            return Err(malformed("no host"));
        }
        // `Url::parse` is lenient about an extra slash: it reads `http:///v1`
        // as host `v1` with an empty path, so `host_str()` alone accepts it —
        // and the `/v1` the operator meant as a path prefix is then silently
        // dropped from every request URL. Require the authority to be written
        // explicitly instead of inferred.
        let authority = url
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        if authority.is_empty() || authority.starts_with('/') {
            return Err(malformed("no host between `://` and the path"));
        }

        let mut model = None;
        let mut timeout = DEFAULT_TIMEOUT;
        let mut max_new_tokens = UPSTREAM_DEFAULT_MAX_NEW_TOKENS;
        // Percent-decode through the URL API rather than splitting the raw
        // string: a model name written by any standard URL builder arrives
        // encoded (`Qwen%2FQwen3`), and forwarding those bytes literally asks
        // the upstream for a model it has never heard of.
        let decoded_query = query
            .map(|query| {
                reqwest::Url::parse(&format!("http://q/?{query}"))
                    .map(|url| {
                        url.query_pairs()
                            .map(|(key, value)| (key.into_owned(), value.into_owned()))
                            .collect::<Vec<_>>()
                    })
                    .map_err(|error| invalid(format!("invalid query string `{query}`: {error}")))
            })
            .transpose()?
            .unwrap_or_default();
        for (key, value) in decoded_query {
            if key.is_empty() {
                continue;
            }
            let (key, value) = (key.as_str(), value.as_str());
            match key {
                "model" => {
                    if value.is_empty() {
                        return Err(invalid("`model` must not be empty".to_owned()));
                    }
                    model = Some(value.to_owned());
                }
                "timeout_ms" => {
                    let millis: u64 = value.parse().map_err(|_| {
                        invalid(format!("`timeout_ms` must be an integer, got `{value}`"))
                    })?;
                    let requested = Duration::from_millis(millis);
                    if requested.is_zero() || requested > MAX_TIMEOUT {
                        return Err(invalid(format!(
                            "`timeout_ms` must be between 1 and {}, got {millis}",
                            MAX_TIMEOUT.as_millis()
                        )));
                    }
                    timeout = requested;
                }
                "max_new_tokens" => {
                    let requested: usize = value.parse().map_err(|_| {
                        invalid(format!(
                            "`max_new_tokens` must be an integer, got `{value}`"
                        ))
                    })?;
                    if requested == 0 || requested > UPSTREAM_MAX_NEW_TOKENS {
                        return Err(invalid(format!(
                            "`max_new_tokens` must be between 1 and {UPSTREAM_MAX_NEW_TOKENS}, got {requested}"
                        )));
                    }
                    max_new_tokens = requested;
                }
                other => {
                    return Err(invalid(format!(
                        "unknown query parameter `{other}`; supported: `model`, `timeout_ms`, `max_new_tokens`"
                    )))
                }
            }
        }

        Ok(Some(Self {
            base_url: url.to_owned(),
            model: model.unwrap_or_else(|| alias.to_owned()),
            timeout,
            max_new_tokens,
        }))
    }

    fn url(&self, suffix: &str) -> String {
        format!("{}{suffix}", self.base_url)
    }
}

/// Resolve the bearer token for an alias, per-alias variable first.
///
/// The returned value is only ever placed in an `Authorization` header; it is
/// never logged, and never round-trips into an error message.
fn api_key_for(alias: &str) -> Option<String> {
    let suffix = alias
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    env::var(format!("{API_KEY_ENV_PREFIX}{suffix}"))
        .or_else(|_| env::var(API_KEY_ENV_FALLBACK))
        .ok()
        .map(|key| key.trim().to_owned())
        .filter(|key| !key.is_empty())
}

/// The host generation request, mirroring `candle_llm_runtime`'s private
/// `GenerationRequest` field for field.
///
/// It is deliberately a separate type rather than a shared one: this backend
/// must accept exactly the same request envelope the native runtime accepts, and
/// keeping its own copy means a future native-only field cannot silently change
/// what gets forwarded to a third-party server.
#[derive(Debug, Default, Deserialize, Serialize)]
struct HostGenerationRequest {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    /// Forwarded verbatim rather than narrowed to the native runtime's
    /// `ChatTurn`. An agentic conversation replays assistant turns that carry
    /// `tool_calls` and no content, plus `tool` turns carrying
    /// `tool_call_id` — narrowing drops exactly the history the upstream needs
    /// to continue the conversation, so the turn after any tool call loses its
    /// context.
    messages: Option<Vec<Value>>,
    #[serde(default)]
    max_new_tokens: Option<usize>,
    /// `f64`, unlike the native runtime's `f32`. Narrowing is right there — the
    /// sampler works in `f32` — but here the value is only ever re-serialized
    /// into an outbound JSON body, and a round trip through `f32` turns a
    /// client's `0.9` into `0.8999999761581421`.
    #[serde(default)]
    temperature: Option<f64>,
    #[serde(default)]
    top_p: Option<f64>,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    stop: Option<Vec<String>>,
    #[serde(default)]
    json_schema: Option<String>,
    /// Tool schemas, forwarded verbatim. The native runtime ignores these — it
    /// has no tool-aware chat template, so `guest-openai` recovers tool calls by
    /// parsing the model's text output. An upstream server does have one, and
    /// telling it about the tools is what makes it emit real `tool_calls`
    /// instead of hoping the prompt described them.
    #[serde(default)]
    tools: Option<Value>,
    #[serde(default)]
    tool_choice: Option<Value>,
}

/// An OpenAI-compatible upstream bound to one mesh alias.
pub(crate) struct UpstreamOpenAiRuntime {
    alias: String,
    endpoint: UpstreamEndpoint,
    api_key: Option<String>,
    client: reqwest::blocking::Client,
}

impl UpstreamOpenAiRuntime {
    /// Claim a binding whose `path` uses the `openai:` scheme.
    ///
    /// `Ok(None)` means "not mine" — the caller keeps probing the on-disk
    /// loaders. Nothing here touches the network: an unreachable upstream is a
    /// per-request failure, not a boot failure, so a node still starts when a
    /// peer inference server is temporarily down.
    pub(crate) fn try_load(alias: &str, path: &str) -> Result<Option<Self>, UpstreamError> {
        let Some(endpoint) = UpstreamEndpoint::parse(alias, path)? else {
            return Ok(None);
        };
        // `reqwest::blocking` spins its own runtime on a private thread. That is
        // safe here because inference executes on the scheduler's dedicated OS
        // thread (`AcceleratorScheduler::new`'s `tachyon-*-dispatcher`), never
        // inside a tokio worker.
        let client = reqwest::blocking::Client::builder()
            .timeout(endpoint.timeout)
            .build()
            .map_err(|error| UpstreamError::InvalidBinding {
                alias: alias.to_owned(),
                detail: format!("could not build the upstream HTTP client: {error}"),
            })?;
        Ok(Some(Self {
            alias: alias.to_owned(),
            api_key: api_key_for(alias),
            endpoint,
            client,
        }))
    }

    pub(crate) fn endpoint(&self) -> &UpstreamEndpoint {
        &self.endpoint
    }

    /// Human-readable execution target for telemetry: the upstream is remote
    /// compute, so it must never be recorded as local `cpu`/`gpu` execution.
    pub(crate) fn executed_on(&self) -> String {
        format!("upstream:{}", self.endpoint.base_url)
    }

    /// Translate one host request into an OpenAI chat-completions body.
    fn chat_body(&self, data: &[u8], stream: bool) -> Result<Value, UpstreamError> {
        // Bounded-input contract, enforced before UTF-8 or JSON parsing so an
        // oversized request cannot force a large allocation on every co-batched
        // thread. The native runtime derives its byte cap from the checkpoint's
        // context window; an upstream's window is not ours to know, so this
        // uses the same absolute ceiling that bounds the derivation.
        if data.len() > MAX_PROMPT_BYTES_CEILING {
            return Err(UpstreamError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!(
                    "prompt bytes {} exceed limit {MAX_PROMPT_BYTES_CEILING}",
                    data.len()
                ),
            });
        }
        let raw = std::str::from_utf8(data).map_err(|error| UpstreamError::InvalidRequest {
            alias: self.alias.clone(),
            detail: format!("prompt tensor must be valid UTF-8: {error}"),
        })?;

        // Same envelope rule as the native runtime: a JSON object is a
        // structured request, anything else is a raw prompt string.
        let request = if raw.trim_start().starts_with('{') {
            serde_json::from_str::<HostGenerationRequest>(raw).map_err(|error| {
                UpstreamError::InvalidRequest {
                    alias: self.alias.clone(),
                    detail: format!("invalid JSON generation request: {error}"),
                }
            })?
        } else {
            HostGenerationRequest {
                prompt: Some(raw.to_owned()),
                ..HostGenerationRequest::default()
            }
        };

        let messages = match (request.messages, request.prompt) {
            // `messages` wins over `prompt`, matching the native runtime, and
            // is forwarded verbatim so the upstream applies its own chat
            // template — the model lives over there, so its template does too.
            (Some(messages), _) if !messages.is_empty() => messages,
            (_, Some(prompt)) => vec![json!({"role": "user", "content": prompt})],
            _ => {
                return Err(UpstreamError::InvalidRequest {
                    alias: self.alias.clone(),
                    detail: "generation request must carry `messages` or `prompt`".to_owned(),
                })
            }
        };

        // A generation budget is always sent, so an absent field cannot leave
        // the upstream's own (possibly unlimited) default in charge. The bound
        // is this binding's, not the native runtime's: see
        // `UPSTREAM_MAX_NEW_TOKENS` for why the two differ.
        let ceiling = self.endpoint.max_new_tokens;
        let max_new_tokens = request.max_new_tokens.unwrap_or(ceiling);
        if max_new_tokens == 0 || max_new_tokens > ceiling {
            return Err(UpstreamError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!(
                    "max_new_tokens {max_new_tokens} must be between 1 and {ceiling} for this upstream binding (raise it with `?max_new_tokens=` on the binding path, up to {UPSTREAM_MAX_NEW_TOKENS})"
                ),
            });
        }

        let mut body = Map::new();
        body.insert("model".to_owned(), json!(self.endpoint.model));
        body.insert("messages".to_owned(), Value::Array(messages));
        body.insert("stream".to_owned(), json!(stream));
        body.insert("max_tokens".to_owned(), json!(max_new_tokens));
        if let Some(temperature) = request.temperature {
            body.insert("temperature".to_owned(), json!(temperature));
        }
        if let Some(top_p) = request.top_p {
            body.insert("top_p".to_owned(), json!(top_p));
        }
        if let Some(seed) = request.seed {
            body.insert("seed".to_owned(), json!(seed));
        }
        if let Some(stop) = request.stop.filter(|stop| !stop.is_empty()) {
            body.insert("stop".to_owned(), json!(stop));
        }
        if let Some(tools) = request.tools.filter(|tools| !is_empty_json(tools)) {
            body.insert("tools".to_owned(), tools);
        }
        if let Some(tool_choice) = request.tool_choice {
            body.insert("tool_choice".to_owned(), tool_choice);
        }
        if let Some(schema) = request.json_schema {
            // The host's `json_schema` constrains decoding. Locally that
            // compiles to an FSM; upstream the equivalent lever is
            // `response_format`, which vLLM and llama.cpp both implement.
            let schema: Value =
                serde_json::from_str(&schema).map_err(|error| UpstreamError::InvalidRequest {
                    alias: self.alias.clone(),
                    detail: format!("invalid `json_schema`: {error}"),
                })?;
            body.insert(
                "response_format".to_owned(),
                json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": "tachyon_response",
                        "schema": schema,
                        "strict": true,
                    }
                }),
            );
        }

        Ok(Value::Object(body))
    }

    fn post(
        &self,
        suffix: &str,
        body: &Value,
    ) -> Result<reqwest::blocking::Response, UpstreamError> {
        let url = self.endpoint.url(suffix);
        let mut request = self.client.post(&url).json(body);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        let response = request.send().map_err(|error| UpstreamError::Transport {
            alias: self.alias.clone(),
            endpoint: url.clone(),
            detail: error.to_string(),
        })?;

        let status = response.status();
        if !status.is_success() {
            // Bound the read itself, not just the excerpt: `Response::text()`
            // would allocate the whole error page first, so a broken upstream
            // returning a huge body could exhaust host memory before the
            // truncation below ever ran.
            let mut raw = Vec::new();
            let _ = std::io::copy(
                &mut std::io::Read::take(response, MAX_ERROR_BODY_BYTES),
                &mut raw,
            );
            let body = String::from_utf8_lossy(&raw).chars().collect::<String>();
            return Err(UpstreamError::Status {
                alias: self.alias.clone(),
                endpoint: url,
                status: status.as_u16(),
                body,
            });
        }
        Ok(response)
    }

    /// Run one buffered generation and return the assistant text as bytes,
    /// matching the native runtime's output contract exactly.
    pub(crate) fn generate(&self, prompts: &[&[u8]]) -> Result<Vec<u8>, UpstreamError> {
        let [prompt] = prompts else {
            return Err(UpstreamError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!(
                    "upstream generation takes exactly one prompt, got {}",
                    prompts.len()
                ),
            });
        };
        let body = self.chat_body(prompt, false)?;
        let response = self.post("/chat/completions", &body)?;
        let payload: Value = read_json(&self.alias, response)?;
        let message = payload
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .ok_or_else(|| UpstreamError::MalformedResponse {
                alias: self.alias.clone(),
                detail: "response has no `choices[0].message` object".to_owned(),
            })?;
        let content = message.get("content").and_then(Value::as_str);

        // A tool call carries `content: null`, so requiring a content string
        // would turn every successful tool call into a malformed-response
        // error. Re-serialize it into the envelope `guest-openai`'s `json`
        // tool-call parser reads back (`{"content": …, "tool_calls": […]}`),
        // which is the only channel this backend has: the host contract is
        // "generation returns text", with tool-call recovery done downstream.
        if let Some(tool_calls) = message
            .get("tool_calls")
            .filter(|tool_calls| !is_empty_json(tool_calls))
        {
            let envelope = json!({
                "content": content.unwrap_or_default(),
                "tool_calls": tool_calls.clone(),
            });
            return Ok(envelope.to_string().into_bytes());
        }

        let text = content.ok_or_else(|| UpstreamError::MalformedResponse {
            alias: self.alias.clone(),
            detail: "response has no `choices[0].message.content` string and no `tool_calls`"
                .to_owned(),
        })?;
        Ok(text.as_bytes().to_vec())
    }

    /// Stream one generation, invoking `on_token` per SSE delta so the mesh's
    /// own `/ai/v1` stream keeps a real time-to-first-token.
    pub(crate) fn generate_streaming(
        &self,
        prompts: &[&[u8]],
        on_token: &mut dyn FnMut(&str),
    ) -> Result<(), UpstreamError> {
        let [prompt] = prompts else {
            return Err(UpstreamError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!(
                    "upstream streaming takes exactly one prompt, got {}",
                    prompts.len()
                ),
            });
        };
        let body = self.chat_body(prompt, true)?;
        // When the request offered tools, content is accumulated rather than
        // streamed. The downstream `json` parser is anchored to a whole-output
        // JSON value, so streaming prose and *then* a tool-call envelope would
        // produce `prose{"content":…}` — unparseable, and the structured call
        // would be handed back as literal assistant text. Buffering costs
        // time-to-first-token only on requests that can produce a call.
        let buffer_content = body.get("tools").is_some();
        let mut buffered_content = String::new();
        let response = self.post("/chat/completions", &body)?;

        // Bound the whole stream, so an upstream that never terminates cannot
        // grow the reader without limit.
        let mut reader = std::io::BufReader::new(std::io::Read::take(response, MAX_STREAM_BYTES));
        let mut line = String::new();
        let mut saw_done = false;
        let mut streamed_tool_calls = StreamedToolCalls::default();
        loop {
            line.clear();
            let read = reader
                .read_line(&mut line)
                .map_err(|error| UpstreamError::Transport {
                    alias: self.alias.clone(),
                    endpoint: self.endpoint.url("/chat/completions"),
                    detail: format!("failed to read the upstream stream: {error}"),
                })?;
            if read == 0 {
                break;
            }
            // `read_line` has already grown the buffer to the newline, so this
            // catches the frame *after* one allocation bounded by
            // `MAX_STREAM_BYTES` — it exists to stop a stream of oversized
            // frames, not the first one.
            if line.len() > MAX_SSE_FRAME_BYTES {
                return Err(UpstreamError::MalformedResponse {
                    alias: self.alias.clone(),
                    detail: format!(
                        "upstream SSE frame exceeds the {MAX_SSE_FRAME_BYTES}-byte limit"
                    ),
                });
            }
            let Some(payload) = line.trim().strip_prefix("data:") else {
                // Blank separator lines and SSE comments carry no delta.
                continue;
            };
            let payload = payload.trim();
            if payload == "[DONE]" {
                saw_done = true;
                break;
            }
            // A malformed frame mid-stream must not discard the tokens already
            // delivered; skip it and keep reading.
            let Ok(frame) = serde_json::from_str::<Value>(payload) else {
                continue;
            };
            // An upstream that committed HTTP 200 and then failed reports it
            // in-band. Without this the frame carries no delta, gets skipped,
            // and `[DONE]` makes the whole request look successful — so the
            // caller receives an empty generation and telemetry agrees.
            if let Some(error) = frame.get("error").filter(|error| !error.is_null()) {
                let detail = error
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| error.to_string());
                return Err(UpstreamError::MalformedResponse {
                    alias: self.alias.clone(),
                    detail: format!(
                        "upstream reported an error mid-stream: {}",
                        detail.chars().take(512).collect::<String>()
                    ),
                });
            }
            let Some(delta) = frame
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("delta"))
            else {
                continue;
            };
            if let Some(content) = delta
                .get("content")
                .and_then(Value::as_str)
                .filter(|content| !content.is_empty())
            {
                if buffer_content {
                    buffered_content.push_str(content);
                } else {
                    on_token(content);
                }
            }
            // A streamed tool call arrives as `delta.tool_calls` fragments with
            // no content at all. Dropping them would make the whole request
            // look like a model that answered with silence, so accumulate and
            // emit them as the same envelope the buffered path returns.
            if let Some(fragments) = delta.get("tool_calls").and_then(Value::as_array) {
                for fragment in fragments {
                    streamed_tool_calls.absorb(fragment);
                }
            }
        }

        // One envelope carrying both, so the downstream anchored parser sees a
        // whole-output JSON value. When no call materialised, the buffered
        // prose is emitted as ordinary text.
        match streamed_tool_calls.finish() {
            Some(tool_calls) => {
                on_token(
                    &json!({"content": buffered_content, "tool_calls": tool_calls}).to_string(),
                );
            }
            None if !buffered_content.is_empty() => on_token(&buffered_content),
            None => {}
        }

        // A clean EOF without `[DONE]` is a truncated generation, not a
        // completed one — the upstream restarted mid-stream, or returned a
        // non-SSE success body. Reporting success here would hand the caller
        // silently truncated code and record the request as healthy.
        if !saw_done {
            return Err(UpstreamError::MalformedResponse {
                alias: self.alias.clone(),
                detail: "upstream stream ended before the `[DONE]` sentinel".to_owned(),
            });
        }
        Ok(())
    }

    /// Forward a single embedding request to the upstream `/embeddings` route.
    pub(crate) fn embed(&self, input: &str) -> Result<Vec<f32>, UpstreamError> {
        let body = json!({"model": self.endpoint.model, "input": input});
        let response = self.post("/embeddings", &body)?;
        let payload: Value = read_json(&self.alias, response)?;
        let embedding = payload
            .get("data")
            .and_then(Value::as_array)
            .and_then(|data| data.first())
            .and_then(|entry| entry.get("embedding"))
            .and_then(Value::as_array)
            .ok_or_else(|| UpstreamError::MalformedResponse {
                alias: self.alias.clone(),
                detail: "response has no `data[0].embedding` array".to_owned(),
            })?;
        // A zero-dimensional embedding is not a usable answer: the route would
        // return HTTP 200 and every vector-index consumer would reject it later
        // as a dimension mismatch, far from the cause.
        if embedding.is_empty() {
            return Err(UpstreamError::MalformedResponse {
                alias: self.alias.clone(),
                detail: "`data[0].embedding` is empty".to_owned(),
            });
        }
        embedding
            .iter()
            .map(|value| {
                let value = value
                    .as_f64()
                    .ok_or_else(|| UpstreamError::MalformedResponse {
                        alias: self.alias.clone(),
                        detail: "`data[0].embedding` contains a non-numeric entry".to_owned(),
                    })?;
                // `1e39 as f32` is `inf`, and an infinite component turns every
                // downstream cosine similarity into NaN, silently corrupting
                // result ordering. Reject rather than narrow.
                let narrowed = value as f32;
                if !narrowed.is_finite() {
                    return Err(UpstreamError::MalformedResponse {
                        alias: self.alias.clone(),
                        detail: format!(
                            "`data[0].embedding` contains {value}, which is not a finite f32"
                        ),
                    });
                }
                Ok(narrowed)
            })
            .collect()
    }
}

/// `true` for JSON that carries nothing worth forwarding: absent, null, or an
/// empty array. Lets an explicitly-empty `tools: []` be dropped rather than
/// sent, which some upstreams reject.
fn is_empty_json(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(items) => items.is_empty(),
        _ => false,
    }
}

/// Reassembles tool calls streamed as OpenAI SSE fragments.
///
/// A streamed tool call is spread across frames: the first carries `id`/`type`
/// and the function name, later ones append `function.arguments` in pieces.
/// Fragments are keyed by their `index`, which is the only field guaranteed to
/// identify which call a fragment belongs to.
#[derive(Default)]
struct StreamedToolCalls {
    /// `(index, id, name, accumulated arguments)`, in first-seen order so the
    /// emitted array preserves the upstream's own ordering.
    calls: Vec<(u64, String, String, String)>,
}

impl StreamedToolCalls {
    fn absorb(&mut self, fragment: &Value) {
        let index = fragment.get("index").and_then(Value::as_u64).unwrap_or(0);
        let slot = match self.calls.iter_mut().find(|(known, ..)| *known == index) {
            Some(slot) => slot,
            None => {
                self.calls
                    .push((index, String::new(), String::new(), String::new()));
                self.calls.last_mut().expect("just pushed")
            }
        };
        if let Some(id) = fragment.get("id").and_then(Value::as_str) {
            slot.1 = id.to_owned();
        }
        let function = fragment.get("function");
        if let Some(name) = function
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
        {
            slot.2 = name.to_owned();
        }
        if let Some(arguments) = function
            .and_then(|function| function.get("arguments"))
            .and_then(Value::as_str)
        {
            slot.3.push_str(arguments);
        }
    }

    /// The reassembled calls, or `None` when the stream carried none. Calls
    /// without a function name are dropped: a fragment stream that never
    /// named its function is not a call anyone can dispatch.
    fn finish(self) -> Option<Value> {
        let calls = self
            .calls
            .into_iter()
            .filter(|(_, _, name, _)| !name.is_empty())
            .map(|(_, id, name, arguments)| {
                let arguments = if arguments.is_empty() {
                    "{}".to_owned()
                } else {
                    arguments
                };
                let mut call = Map::new();
                if !id.is_empty() {
                    call.insert("id".to_owned(), json!(id));
                }
                call.insert("type".to_owned(), json!("function"));
                call.insert(
                    "function".to_owned(),
                    json!({"name": name, "arguments": arguments}),
                );
                Value::Object(call)
            })
            .collect::<Vec<_>>();
        (!calls.is_empty()).then_some(Value::Array(calls))
    }
}

/// Read a bounded JSON body, so a hostile or broken upstream cannot pull the
/// host into an unbounded allocation.
fn read_json(alias: &str, response: reqwest::blocking::Response) -> Result<Value, UpstreamError> {
    let mut body = Vec::new();
    std::io::copy(
        &mut std::io::Read::take(response, MAX_RESPONSE_BYTES + 1),
        &mut body,
    )
    .map_err(|error| UpstreamError::MalformedResponse {
        alias: alias.to_owned(),
        detail: format!("failed to read the upstream response body: {error}"),
    })?;
    if body.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(UpstreamError::MalformedResponse {
            alias: alias.to_owned(),
            detail: format!("response body exceeds the {MAX_RESPONSE_BYTES}-byte limit"),
        });
    }
    serde_json::from_slice(&body).map_err(|error| UpstreamError::MalformedResponse {
        alias: alias.to_owned(),
        detail: format!("response was not JSON: {error}"),
    })
}

/// A canned HTTP/1.1 server that records the requests it received and replies
/// with a fixed response. Real sockets rather than a mocked client, so tests
/// exercise the actual reqwest round trip and SSE framing.
///
/// Lives at module scope (not inside `mod tests`) so `ai_inference`'s own tests
/// can drive the backend end to end through `BackendModel`.
#[cfg(test)]
pub(crate) struct FakeUpstream {
    base_url: String,
    requests: std::sync::mpsc::Receiver<(String, String)>,
}

#[cfg(test)]
impl FakeUpstream {
    pub(crate) fn start(status_line: &str, content_type: &str, body: &str) -> Self {
        Self::start_many(status_line, content_type, body, 1)
    }

    /// `connections` bounds how many requests the server answers before it
    /// stops accepting, so a concurrent batch can be served from one fixture.
    pub(crate) fn start_many(
        status_line: &str,
        content_type: &str,
        body: &str,
        connections: usize,
    ) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("port should bind");
        let base_url = format!(
            "http://{}/v1",
            listener
                .local_addr()
                .expect("listener should have an address")
        );
        let (tx, requests) = std::sync::mpsc::channel();
        let response = format!(
            "{status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );

        // Detached: the accept loop parks on `accept()` if a test makes fewer
        // requests than it allowed for, so joining it on drop would hang.
        std::thread::spawn(move || {
            use std::io::{BufRead, Read, Write};
            for _ in 0..connections {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                let tx = tx.clone();
                let response = response.clone();
                // One thread per connection: a concurrent batch would otherwise
                // serialise against this accept loop and hide the very
                // parallelism the test is checking.
                std::thread::spawn(move || {
                    let mut reader = std::io::BufReader::new(stream);

                    // Request line plus headers, stopping at the blank separator.
                    let mut target = String::new();
                    let mut content_length = 0usize;
                    let mut line = String::new();
                    while reader.read_line(&mut line).unwrap_or(0) > 0 {
                        if line == "\r\n" {
                            break;
                        }
                        if target.is_empty() {
                            target = line.trim().to_owned();
                        }
                        if let Some(value) =
                            line.to_ascii_lowercase().strip_prefix("content-length:")
                        {
                            content_length = value.trim().parse().unwrap_or(0);
                        }
                        line.clear();
                    }

                    let mut body = vec![0u8; content_length];
                    let _ = reader.read_exact(&mut body);
                    let _ = tx.send((target, String::from_utf8_lossy(&body).into_owned()));

                    let mut stream = reader.into_inner();
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                });
            }
        });

        Self { base_url, requests }
    }

    pub(crate) fn binding(&self) -> String {
        format!("{UPSTREAM_SCHEME}{}", self.base_url)
    }

    /// The (request-line, parsed body) pair of the next request received.
    pub(crate) fn received(&self) -> (String, Value) {
        let (target, body) = self
            .requests
            .recv_timeout(Duration::from_secs(10))
            .expect("the upstream should have received a request");
        let body = serde_json::from_str(&body).unwrap_or(Value::Null);
        (target, body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_upstream_paths_are_not_claimed() {
        assert!(UpstreamEndpoint::parse("m", "/models/llama")
            .expect("a filesystem path is not an error")
            .is_none());
        assert!(UpstreamEndpoint::parse("m", "mock:demo")
            .expect("a mock path is not an error")
            .is_none());
    }

    #[test]
    fn upstream_defaults_the_model_to_the_alias() {
        let endpoint = UpstreamEndpoint::parse("qwen-coder", "openai:http://127.0.0.1:8080/v1")
            .expect("binding should parse")
            .expect("binding should be claimed");
        assert_eq!(endpoint.base_url, "http://127.0.0.1:8080/v1");
        assert_eq!(endpoint.model, "qwen-coder");
        assert_eq!(endpoint.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn upstream_reads_model_and_timeout_overrides() {
        let endpoint = UpstreamEndpoint::parse(
            "coder",
            "openai:https://gpu.lan:8000/v1/?model=qwen3-coder-30b&timeout_ms=120000",
        )
        .expect("binding should parse")
        .expect("binding should be claimed");
        // The trailing slash is stripped so request URLs never double up.
        assert_eq!(endpoint.base_url, "https://gpu.lan:8000/v1");
        assert_eq!(endpoint.model, "qwen3-coder-30b");
        assert_eq!(endpoint.timeout, Duration::from_millis(120_000));
    }

    #[test]
    fn upstream_rejects_a_non_http_url() {
        let error = UpstreamEndpoint::parse("m", "openai:127.0.0.1:8080")
            .expect_err("a scheme-less URL must be rejected");
        assert!(
            error.to_string().contains("http://"),
            "error should name the accepted schemes, got: {error}"
        );
    }

    #[test]
    fn upstream_rejects_a_missing_host() {
        assert!(UpstreamEndpoint::parse("m", "openai:http://").is_err());
        // Authority-less but textually non-empty after the scheme: a prefix
        // check would let this through and reqwest would only complain on the
        // first request.
        let error = UpstreamEndpoint::parse("m", "openai:http:///v1")
            .expect_err("an authority-less URL must be rejected at load");
        assert!(
            error.to_string().contains("no host"),
            "the error should say the host is missing, got: {error}"
        );
    }

    #[test]
    fn max_new_tokens_is_bounded_by_the_bindings_own_ceiling() {
        let backend = runtime("coder", "openai:http://127.0.0.1:8080/v1");

        // Absent: a budget is always sent, never omitted — and it is the
        // upstream default, not the native runtime's 256-token cap, which would
        // truncate an agentic completion mid-function.
        let body = backend
            .chat_body(br#"{"prompt":"p"}"#, false)
            .expect("request should map");
        assert_eq!(body["max_tokens"], UPSTREAM_DEFAULT_MAX_NEW_TOKENS);

        // Out of range in either direction is rejected before any round trip.
        for max_new_tokens in [0, UPSTREAM_DEFAULT_MAX_NEW_TOKENS + 1] {
            let request = format!(r#"{{"prompt":"p","max_new_tokens":{max_new_tokens}}}"#);
            let error = backend
                .chat_body(request.as_bytes(), false)
                .expect_err("an out-of-range max_new_tokens must be rejected");
            assert!(matches!(error, UpstreamError::InvalidRequest { .. }));
        }

        let body = backend
            .chat_body(
                format!(r#"{{"prompt":"p","max_new_tokens":{UPSTREAM_DEFAULT_MAX_NEW_TOKENS}}}"#)
                    .as_bytes(),
                false,
            )
            .expect("the ceiling itself is valid");
        assert_eq!(body["max_tokens"], UPSTREAM_DEFAULT_MAX_NEW_TOKENS);
    }

    #[test]
    fn a_binding_can_raise_its_own_generation_ceiling() {
        let backend = runtime(
            "coder",
            "openai:http://127.0.0.1:8080/v1?max_new_tokens=6000",
        );

        let body = backend
            .chat_body(br#"{"prompt":"p"}"#, false)
            .expect("request should map");
        assert_eq!(body["max_tokens"], 6000);

        let body = backend
            .chat_body(br#"{"prompt":"p","max_new_tokens":5000}"#, false)
            .expect("a request under the binding ceiling is valid");
        assert_eq!(body["max_tokens"], 5000);

        assert!(backend
            .chat_body(br#"{"prompt":"p","max_new_tokens":6001}"#, false)
            .is_err());

        // The per-binding override is itself bounded, so a typo cannot make a
        // request effectively unlimited.
        assert!(UpstreamEndpoint::parse(
            "coder",
            &format!(
                "openai:http://h:1/v1?max_new_tokens={}",
                UPSTREAM_MAX_NEW_TOKENS + 1
            )
        )
        .is_err());
        assert!(UpstreamEndpoint::parse("coder", "openai:http://h:1/v1?max_new_tokens=0").is_err());
    }

    #[test]
    fn upstream_rejects_unknown_and_out_of_range_query_parameters() {
        assert!(UpstreamEndpoint::parse("m", "openai:http://h:1/v1?temperature=0.2").is_err());
        assert!(UpstreamEndpoint::parse("m", "openai:http://h:1/v1?timeout_ms=0").is_err());
        assert!(UpstreamEndpoint::parse("m", "openai:http://h:1/v1?timeout_ms=nope").is_err());
        assert!(UpstreamEndpoint::parse(
            "m",
            &format!(
                "openai:http://h:1/v1?timeout_ms={}",
                MAX_TIMEOUT.as_millis() + 1
            )
        )
        .is_err());
    }

    fn runtime(alias: &str, path: &str) -> UpstreamOpenAiRuntime {
        UpstreamOpenAiRuntime::try_load(alias, path)
            .expect("binding should load")
            .expect("binding should be claimed")
    }

    #[test]
    fn structured_requests_map_onto_the_openai_schema() {
        let backend = runtime(
            "coder",
            "openai:http://127.0.0.1:8080/v1?model=upstream-name",
        );
        let body = backend
            .chat_body(
                br#"{"messages":[{"role":"user","content":"hi"}],"max_new_tokens":32,
                     "temperature":0.2,"top_p":0.9,"seed":7,"stop":["</s>"]}"#,
                false,
            )
            .expect("request should map");

        assert_eq!(body["model"], "upstream-name");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "hi");
        // `max_new_tokens` is the host's name for it; upstream expects
        // `max_tokens`, and forwarding the host name would silently uncap.
        assert_eq!(body["max_tokens"], 32);
        assert!(body.get("max_new_tokens").is_none());
        assert_eq!(body["top_p"], 0.9);
        assert_eq!(body["seed"], 7);
        assert_eq!(body["stop"][0], "</s>");
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn raw_prompts_become_a_single_user_turn() {
        let backend = runtime("coder", "openai:http://127.0.0.1:8080/v1");
        let body = backend
            .chat_body(b"write a haiku", true)
            .expect("request should map");
        assert_eq!(body["messages"].as_array().expect("array").len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "write a haiku");
        assert_eq!(body["stream"], true);
        // Absent sampling knobs must stay absent rather than be sent as
        // nulls: some upstreams reject an explicit null temperature.
        assert!(body.get("temperature").is_none());
        assert!(body.get("stop").is_none());
    }

    #[test]
    fn json_schema_becomes_a_response_format() {
        let backend = runtime("coder", "openai:http://127.0.0.1:8080/v1");
        let body = backend
            .chat_body(
                br#"{"prompt":"p","json_schema":"{\"type\":\"object\"}"}"#,
                false,
            )
            .expect("request should map");
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(
            body["response_format"]["json_schema"]["schema"]["type"],
            "object"
        );
    }

    #[test]
    fn a_request_without_prompt_or_messages_is_rejected() {
        let backend = runtime("coder", "openai:http://127.0.0.1:8080/v1");
        let error = backend
            .chat_body(br#"{"max_new_tokens":8}"#, false)
            .expect_err("an empty request must be rejected");
        assert!(matches!(error, UpstreamError::InvalidRequest { .. }));
    }

    #[test]
    fn telemetry_names_the_upstream_rather_than_a_local_accelerator() {
        let backend = runtime("coder", "openai:http://127.0.0.1:8080/v1");
        assert_eq!(backend.executed_on(), "upstream:http://127.0.0.1:8080/v1");
    }

    #[test]
    fn buffered_generation_round_trips_through_a_real_upstream() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"choices":[{"message":{"role":"assistant","content":"fn main() {}"},
                "finish_reason":"stop"}]}"#,
        );
        let backend = runtime("coder", &upstream.binding());

        let output = backend
            .generate(&[br#"{"messages":[{"role":"user","content":"write main"}]}"#])
            .expect("generation should round trip");
        assert_eq!(output, b"fn main() {}".to_vec());

        let (target, body) = upstream.received();
        assert!(
            target.starts_with("POST /v1/chat/completions "),
            "unexpected request line: {target}"
        );
        // The alias is the default upstream model name.
        assert_eq!(body["model"], "coder");
        assert_eq!(body["messages"][0]["content"], "write main");
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn streaming_generation_emits_one_token_per_sse_delta() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"fn \"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"main\"}}]}\n\n",
                // An empty delta (the usual role-only opening frame) must not
                // emit a token, and a malformed frame must not abort the stream.
                "data: {\"choices\":[{\"delta\":{\"content\":\"\"}}]}\n\n",
                "data: {not json}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"()\"}}]}\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut tokens = Vec::new();
        backend
            .generate_streaming(&[b"write main"], &mut |token| tokens.push(token.to_owned()))
            .expect("streaming should complete");
        assert_eq!(tokens, vec!["fn ", "main", "()"]);

        let (_, body) = upstream.received();
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn an_upstream_error_status_surfaces_with_its_body() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 503 Service Unavailable",
            "application/json",
            r#"{"error":{"message":"model is loading"}}"#,
        );
        let backend = runtime("coder", &upstream.binding());

        let error = backend
            .generate(&[b"hello"])
            .expect_err("a 503 must not be reported as generated text");
        match error {
            UpstreamError::Status { status, body, .. } => {
                assert_eq!(status, 503);
                assert!(
                    body.contains("model is loading"),
                    "the upstream's own explanation must reach the operator, got: {body}"
                );
            }
            other => panic!("expected a status error, got: {other}"),
        }
    }

    #[test]
    fn embeddings_are_forwarded_to_the_upstream_route() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"data":[{"embedding":[0.25,-0.5,1.0]}]}"#,
        );
        let backend = runtime("embed", &upstream.binding());

        let vector = backend.embed("hello").expect("embedding should round trip");
        assert_eq!(vector, vec![0.25, -0.5, 1.0]);

        let (target, body) = upstream.received();
        assert!(
            target.starts_with("POST /v1/embeddings "),
            "unexpected request line: {target}"
        );
        assert_eq!(body["input"], "hello");
    }

    #[test]
    fn a_stream_that_ends_before_done_is_reported_as_truncated() {
        // Two real deltas, then a clean EOF: an upstream restart mid-generation
        // looks exactly like this, and calling it success would hand the caller
        // silently truncated code.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"fn \"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"main\"}}]}\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut tokens = Vec::new();
        let error = backend
            .generate_streaming(&[b"write main"], &mut |token| tokens.push(token.to_owned()))
            .expect_err("a stream without [DONE] is truncated, not complete");
        assert!(matches!(error, UpstreamError::MalformedResponse { .. }));
        assert!(
            error.to_string().contains("[DONE]"),
            "the error should name the missing sentinel, got: {error}"
        );
        // Tokens already forwarded are not retracted; only the outcome changes.
        assert_eq!(tokens, vec!["fn ", "main"]);
    }

    #[test]
    fn a_non_finite_embedding_component_is_rejected() {
        // `1e39 as f32` is `inf`, which would turn every downstream cosine
        // similarity into NaN.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"data":[{"embedding":[0.5,1e39]}]}"#,
        );
        let backend = runtime("embed", &upstream.binding());

        let error = backend
            .embed("hello")
            .expect_err("an infinite component is not a usable embedding");
        assert!(matches!(error, UpstreamError::MalformedResponse { .. }));
    }

    #[test]
    fn an_oversized_error_body_is_truncated_to_the_read_limit() {
        let body = "x".repeat(4 * MAX_ERROR_BODY_BYTES as usize);
        let upstream =
            FakeUpstream::start("HTTP/1.1 500 Internal Server Error", "text/html", &body);
        let backend = runtime("coder", &upstream.binding());

        let error = backend
            .generate(&[b"hello"])
            .expect_err("a 500 must not be reported as generated text");
        match error {
            UpstreamError::Status { body, .. } => assert!(
                body.len() as u64 <= MAX_ERROR_BODY_BYTES,
                "the error excerpt should be bounded by the read limit, got {} bytes",
                body.len()
            ),
            other => panic!("expected a status error, got: {other}"),
        }
    }

    #[test]
    fn tool_schemas_are_forwarded_so_the_upstream_can_apply_its_own_template() {
        let backend = runtime("coder", "openai:http://127.0.0.1:8080/v1");
        let body = backend
            .chat_body(
                br#"{"prompt":"weather?",
                     "tools":[{"type":"function","function":{"name":"get_weather"}}],
                     "tool_choice":"auto"}"#,
                false,
            )
            .expect("request should map");
        assert_eq!(body["tools"][0]["function"]["name"], "get_weather");
        assert_eq!(body["tool_choice"], "auto");

        // An explicitly empty tools array is dropped rather than sent: some
        // upstreams reject `tools: []`.
        let body = backend
            .chat_body(br#"{"prompt":"hi","tools":[]}"#, false)
            .expect("request should map");
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn a_buffered_tool_call_is_returned_instead_of_failing_on_null_content() {
        // A real tool-call response carries `content: null`; requiring a content
        // string would turn every successful tool call into an error.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[
                {"id":"call_1","type":"function",
                 "function":{"name":"read_file","arguments":"{\"path\":\"a.rs\"}"}}]},
                "finish_reason":"tool_calls"}]}"#,
        );
        let backend = runtime("coder", &upstream.binding());

        let output = backend
            .generate(&[b"read a.rs"])
            .expect("a tool call is a successful generation");
        let envelope: Value =
            serde_json::from_slice(&output).expect("output should be the JSON envelope");
        assert_eq!(envelope["tool_calls"][0]["function"]["name"], "read_file");
        assert_eq!(
            envelope["tool_calls"][0]["function"]["arguments"],
            "{\"path\":\"a.rs\"}"
        );
        assert_eq!(envelope["content"], "");
    }

    #[test]
    fn streamed_tool_call_fragments_are_reassembled() {
        // Name arrives in the first fragment, arguments in pieces after it —
        // and no content frame ever arrives, so dropping these would look like
        // a model that answered with silence.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"read_file","arguments":""}}]}}]}"#,
                "\n\n",
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]}}]}"#,
                "\n\n",
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"a.rs\"}"}}]}}]}"#,
                "\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut tokens = Vec::new();
        backend
            .generate_streaming(&[b"read a.rs"], &mut |token| tokens.push(token.to_owned()))
            .expect("streaming should complete");
        assert_eq!(tokens.len(), 1, "expected one envelope, got {tokens:?}");
        let envelope: Value =
            serde_json::from_str(&tokens[0]).expect("the emitted token should be the envelope");
        assert_eq!(envelope["tool_calls"][0]["id"], "call_1");
        assert_eq!(envelope["tool_calls"][0]["function"]["name"], "read_file");
        assert_eq!(
            envelope["tool_calls"][0]["function"]["arguments"],
            "{\"path\":\"a.rs\"}"
        );
    }

    #[test]
    fn a_stream_without_tool_calls_emits_no_envelope() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut tokens = Vec::new();
        backend
            .generate_streaming(&[b"hi"], &mut |token| tokens.push(token.to_owned()))
            .expect("streaming should complete");
        assert_eq!(tokens, vec!["ok"]);
    }

    #[test]
    fn upstream_rejects_credentials_and_fragments_in_the_url() {
        // Userinfo becomes a Basic auth header inside reqwest, bypassing the
        // environment-only credential contract, and `base_url` is echoed in
        // telemetry and errors.
        let error = UpstreamEndpoint::parse("m", "openai:https://user:secret@gpu.lan/v1")
            .expect_err("embedded credentials must be rejected");
        assert!(error.to_string().contains("TACHYON_UPSTREAM_API_KEY"));
        assert!(
            !error.to_string().contains("secret"),
            "the rejection must not echo the secret: {error}"
        );

        // A fragment is never sent, so `#x` + `/chat/completions` would reach
        // the upstream as bare `/v1`.
        assert!(UpstreamEndpoint::parse("m", "openai:http://h:1/v1#proxy").is_err());
    }

    #[test]
    fn upstream_percent_decodes_the_model_query_value() {
        let endpoint = UpstreamEndpoint::parse("m", "openai:http://h:1/v1?model=Qwen%2FQwen3")
            .expect("binding should parse")
            .expect("binding should be claimed");
        // Any standard URL builder encodes the slash; forwarding the raw bytes
        // would ask the upstream for a model it has never heard of.
        assert_eq!(endpoint.model, "Qwen/Qwen3");
    }

    #[test]
    fn an_oversized_prompt_is_rejected_before_parsing() {
        let backend = runtime("coder", "openai:http://127.0.0.1:8080/v1");
        let oversized = vec![b'x'; MAX_PROMPT_BYTES_CEILING + 1];
        let error = backend
            .chat_body(&oversized, false)
            .expect_err("an oversized prompt must be rejected");
        assert!(matches!(error, UpstreamError::InvalidRequest { .. }));
        assert!(error.to_string().contains("prompt bytes"));
    }

    #[test]
    fn multi_turn_tool_history_is_forwarded_unchanged() {
        let backend = runtime("coder", "openai:http://127.0.0.1:8080/v1");
        // The exact replay a client sends on the turn after a tool call: an
        // assistant turn with no content, then a `tool` turn.
        let body = backend
            .chat_body(
                br#"{"messages":[
                    {"role":"user","content":"read a.rs"},
                    {"role":"assistant","tool_calls":[{"id":"call_1","type":"function",
                        "function":{"name":"read_file","arguments":"{}"}}]},
                    {"role":"tool","tool_call_id":"call_1","content":"fn main() {}"}
                ]}"#,
                false,
            )
            .expect("a multi-turn tool conversation must map");
        let messages = body["messages"].as_array().expect("messages array");
        assert_eq!(messages.len(), 3);
        // Narrowing to the native `ChatTurn` would have dropped exactly this.
        assert_eq!(messages[1]["tool_calls"][0]["id"], "call_1");
        assert_eq!(messages[2]["tool_call_id"], "call_1");
    }

    #[test]
    fn an_empty_embedding_is_rejected() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"data":[{"embedding":[]}]}"#,
        );
        let backend = runtime("embed", &upstream.binding());
        let error = backend
            .embed("hello")
            .expect_err("a zero-dimensional embedding is not usable");
        assert!(matches!(error, UpstreamError::MalformedResponse { .. }));
    }

    #[test]
    fn a_mid_stream_error_event_fails_the_request() {
        // HTTP 200 committed, then the upstream reports failure in-band and
        // still sends [DONE]. Skipping the frame would report success.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                "data: {\"error\":{\"message\":\"context length exceeded\"}}\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());
        let error = backend
            .generate_streaming(&[b"hi"], &mut |_| {})
            .expect_err("an in-band error must not be reported as success");
        assert!(error.to_string().contains("context length exceeded"));
    }

    #[test]
    fn tool_enabled_streams_emit_one_parseable_envelope() {
        // Content then tool calls: streaming the prose first and appending the
        // envelope would yield `prose{...}`, which the anchored downstream
        // parser cannot read, losing the call.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                r#"data: {"choices":[{"delta":{"content":"Let me look. "}}]}"#,
                "\n\n",
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"read_file","arguments":"{}"}}]}}]}"#,
                "\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut tokens = Vec::new();
        backend
            .generate_streaming(
                &[br#"{"prompt":"read","tools":[{"type":"function","function":{"name":"read_file"}}]}"#],
                &mut |token| tokens.push(token.to_owned()),
            )
            .expect("streaming should complete");
        assert_eq!(tokens.len(), 1, "expected one envelope, got {tokens:?}");
        let envelope: Value = serde_json::from_str(&tokens[0]).expect("envelope should parse");
        assert_eq!(envelope["content"], "Let me look. ");
        assert_eq!(envelope["tool_calls"][0]["function"]["name"], "read_file");
    }

    #[test]
    fn streams_without_tools_keep_streaming_content() {
        // No tools offered: nothing can become a tool call, so time-to-first-
        // token is preserved.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"b\"}}]}\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());
        let mut tokens = Vec::new();
        backend
            .generate_streaming(&[b"hi"], &mut |token| tokens.push(token.to_owned()))
            .expect("streaming should complete");
        assert_eq!(tokens, vec!["a", "b"]);
    }

    #[test]
    fn a_response_without_choices_is_rejected_rather_than_returned_empty() {
        let upstream = FakeUpstream::start("HTTP/1.1 200 OK", "application/json", r#"{"id":"x"}"#);
        let backend = runtime("coder", &upstream.binding());
        let error = backend
            .generate(&[b"hello"])
            .expect_err("a choice-less response is not a valid generation");
        assert!(matches!(error, UpstreamError::MalformedResponse { .. }));
    }
}
