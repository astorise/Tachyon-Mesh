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

use super::candle_llm_runtime::{
    TokenUsage, HOST_MAX_GENERATION_DEADLINE, MAX_PROMPT_BYTES_CEILING,
};
use super::{StreamEvent, StreamOutcome, StreamSink, ToolCall};
// Named only by the tests now: the backend reaches `StreamControl` through
// `StreamSink::emit`'s return value rather than constructing one.
#[cfg(test)]
use super::StreamControl;

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

impl UpstreamError {
    /// The status this node should answer with when an upstream call fails.
    ///
    /// A remote response keeps its own status: a 401, a 404 or a 429 means
    /// something specific and actionable, and flattening it loses that.
    ///
    /// A transport failure and an unusable body used to yield `None`, on the
    /// reasoning that no server had answered so no status was the provider's to
    /// attribute. But `None` is rendered as 500 `server_error`, which says
    /// *this* node broke — and an agent retrying a 500 against a node that is
    /// working fine retries forever against an upstream that is not. Serving
    /// this alias makes the node a gateway, and 502 is exactly what HTTP has
    /// for a gateway whose upstream did not answer usably. That is a truthful
    /// answer, not an attribution.
    ///
    /// The two genuinely local faults keep `None`: a malformed binding is this
    /// deployment's own configuration, and a rejected request never left.
    pub(crate) fn http_status(&self) -> Option<u16> {
        match self {
            Self::Status { status, .. } => Some(*status),
            Self::Transport { .. } | Self::MalformedResponse { .. } => Some(502),
            Self::InvalidBinding { .. } | Self::InvalidRequest { .. } => None,
        }
    }
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

        // Reject `#` before splitting, not after. Splitting at `?` first hides
        // a fragment written in its normal position — `…/v1?model=foo#bar`
        // leaves the fragment inside the *query* half, so the check below only
        // ever saw `…/v1` and the binding loaded with `#bar` silently folded
        // into the last parameter's value.
        if remainder.contains('#') {
            return Err(invalid(format!(
                "upstream URL `{remainder}` must not contain a `#` fragment: it is never sent on the wire, so request paths and query parameters would silently lose their suffix"
            )));
        }
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
    // A set-but-empty per-alias variable is not an answer, it is an absent
    // secret — which is what optional secret injection produces when the
    // secret is missing. Treating `Ok("")` as a hit skipped the global
    // fallback and sent the request unauthenticated, so each candidate is
    // normalized before it is accepted.
    [
        format!("{API_KEY_ENV_PREFIX}{}", api_key_env_suffix(alias)),
        API_KEY_ENV_FALLBACK.to_owned(),
    ]
    .into_iter()
    .find_map(|name| {
        env::var(name)
            .ok()
            .map(|key| key.trim().to_owned())
            .filter(|key| !key.is_empty())
    })
}

/// The per-alias half of `TACHYON_UPSTREAM_API_KEY_<SUFFIX>`.
///
/// Not injective: every character outside `[A-Za-z0-9]` collapses to `_`, so
/// `vendor-a`, `vendor_a` and `vendor.a` all name the same variable. That is
/// deliberate — an operator types these by hand and should not have to encode
/// punctuation — and it is why [`assert_no_credential_collisions`] refuses a
/// manifest where two upstream aliases would share one.
pub(crate) fn api_key_env_suffix(alias: &str) -> String {
    alias
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Refuse a set of upstream aliases whose credential variables collide.
///
/// Two aliases sharing one `TACHYON_UPSTREAM_API_KEY_<SUFFIX>` means the bearer
/// token an operator provisioned for one third party is also sent to the other.
/// Nothing at request time can detect that — `api_key_for` sees one alias — so
/// it is caught where the whole manifest is visible, and it fails the boot
/// rather than the request.
pub(crate) fn assert_no_credential_collisions<'a>(
    aliases: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let mut claimed: std::collections::HashMap<String, &str> = std::collections::HashMap::new();
    for alias in aliases {
        let suffix = api_key_env_suffix(alias);
        if let Some(previous) = claimed.insert(suffix.clone(), alias) {
            return Err(format!(
                "upstream aliases `{previous}` and `{alias}` both read                  `{API_KEY_ENV_PREFIX}{suffix}`, so one binding's API key would be                  sent to the other's server; rename one of them"
            ));
        }
    }
    Ok(())
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
    /// Wall-clock budget in milliseconds, mirrored from the native runtime.
    /// Not forwarded upstream — it is a *local* slot budget, and no
    /// OpenAI-compatible server takes it — but it bounds this request's own
    /// HTTP timeout. Without the field serde dropped it, so a caller asking
    /// for 1 ms could hold a network slot for the binding's full timeout.
    #[serde(default)]
    max_generation_ms: Option<u64>,
}

/// A completed upstream generation: its bytes, the counts the upstream
/// reported, and why it stopped. The last two are `Option` because "not
/// reported" is a distinct answer from zero or `stop`.
/// One completed upstream generation. Mirrors `InferenceOutput` field for
/// field, so the backend adapter is a move rather than a re-encoding.
#[derive(Debug)]
pub(crate) struct UpstreamGeneration {
    pub(crate) bytes: Vec<u8>,
    pub(crate) usage: Option<TokenUsage>,
    pub(crate) finish_reason: Option<String>,
    pub(crate) tool_calls: Vec<ToolCall>,
}

/// An OpenAI-compatible upstream bound to one mesh alias.
pub(crate) struct UpstreamOpenAiRuntime {
    alias: String,
    endpoint: UpstreamEndpoint,
    api_key: Option<String>,
    client: reqwest::blocking::Client,
}

/// How long a silent upstream is given before this side re-checks that anyone
/// is still listening. Short enough that an abandoned request frees its
/// admission permit promptly, long enough that a busy stream is not paying for
/// a wakeup per frame.
const SSE_LIVENESS_POLL: Duration = Duration::from_millis(250);

/// What one poll of the reader thread found.
enum SseRead {
    Line(String),
    /// Nothing within the poll window — the upstream is thinking, not gone.
    Idle,
    Eof,
    Failed(String),
}

/// Reads SSE lines on a dedicated thread so the caller can keep answering for
/// itself while a blocking read is in flight.
struct SseLineReader {
    lines: std::sync::mpsc::Receiver<Result<String, String>>,
}

impl SseLineReader {
    fn spawn<R: std::io::Read + Send + 'static>(mut reader: std::io::BufReader<R>) -> Self {
        // Depth one: the reader stays one line ahead and then parks. Buffering
        // more would let a fast upstream race ahead of a consumer that has
        // already gone away, which is the situation this whole seam exists to
        // end early.
        let (tx, lines) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("tachyon-upstream-sse".to_owned())
            .spawn(move || {
                loop {
                    let mut line = String::new();
                    // Bound the *read*, not just the result. A plain
                    // `read_line` grows its buffer until a newline, so an
                    // upstream that never sends one could force allocation all
                    // the way to `MAX_STREAM_BYTES` before any size check ran —
                    // and several concurrent streams would multiply that.
                    let read = std::io::Read::take(
                        std::io::Read::by_ref(&mut reader),
                        MAX_SSE_FRAME_BYTES as u64 + 1,
                    )
                    .read_line(&mut line);
                    let message = match read {
                        Ok(0) => return,
                        Ok(_) => Ok(line),
                        Err(error) => {
                            let _ = tx
                                .send(Err(format!("failed to read the upstream stream: {error}")));
                            return;
                        }
                    };
                    // A closed receiver means the caller has stopped caring.
                    // Returning drops the response, which closes the socket.
                    if tx.send(message).is_err() {
                        return;
                    }
                }
            })
            // A host that cannot spawn is already failing; report it as an
            // empty stream rather than panicking inside a request.
            .map_or_else(
                |_| Self {
                    lines: std::sync::mpsc::channel().1,
                },
                |_handle| Self { lines },
            )
    }

    fn next_line(&mut self, poll: Duration) -> SseRead {
        match self.lines.recv_timeout(poll) {
            Ok(Ok(line)) => SseRead::Line(line),
            Ok(Err(detail)) => SseRead::Failed(detail),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => SseRead::Idle,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => SseRead::Eof,
        }
    }
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
    /// Build the upstream request body, and the wall-clock bound this
    /// particular call must respect.
    ///
    /// The caller's `max_generation_ms` is a *local* slot budget — no
    /// OpenAI-compatible server takes such a field — so it never goes on the
    /// wire; it tightens this request's own HTTP timeout instead. The binding's
    /// configured `timeout_ms` remains the ceiling: a request may ask for less
    /// time than the operator allowed, never more.
    fn chat_body(&self, data: &[u8], stream: bool) -> Result<(Value, Duration), UpstreamError> {
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

        let timeout = match request.max_generation_ms {
            None => self.endpoint.timeout,
            Some(millis) => {
                let requested = Duration::from_millis(millis);
                if requested.is_zero() || requested > HOST_MAX_GENERATION_DEADLINE {
                    return Err(UpstreamError::InvalidRequest {
                        alias: self.alias.clone(),
                        detail: format!(
                            "max_generation_ms {millis} must be between 1 and {}",
                            HOST_MAX_GENERATION_DEADLINE.as_millis()
                        ),
                    });
                }
                requested.min(self.endpoint.timeout)
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

        Ok((Value::Object(body), timeout))
    }

    fn post(
        &self,
        suffix: &str,
        body: &Value,
        timeout: Duration,
    ) -> Result<reqwest::blocking::Response, UpstreamError> {
        let url = self.endpoint.url(suffix);
        // Per request, not per client: the binding's `timeout_ms` is the
        // ceiling, and a caller's `max_generation_ms` tightens it for this call
        // alone.
        let mut request = self.client.post(&url).timeout(timeout).json(body);
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
    /// Read an OpenAI `usage` object out of a response or an SSE frame.
    ///
    /// Read opportunistically rather than requested: OpenAI only emits `usage`
    /// on a stream when `stream_options.include_usage` is set, but sending that
    /// field to an upstream that does not know it risks a 400 that would break
    /// streaming outright — a bad trade for a reporting field. Upstreams that
    /// volunteer it (and every upstream on the buffered path) are reported;
    /// the rest report nothing, which the caller renders as an absent `usage`
    /// rather than as zeros.
    /// An absent or null counter is genuinely unreported and reads as zero. A
    /// *present* one that is not a non-negative integer, or one too large for
    /// the `u32` the counters cross the WIT boundary as, is a malformed
    /// response and says so. Both used to be swallowed: `as_u64().unwrap_or(0)`
    /// turned `"100"` into an authoritative zero, and the unchecked cast turned
    /// `4294967297` into `1` — client-visible billing figures, quietly wrong,
    /// with the other counter lending them credibility.
    fn read_usage(payload: &Value) -> Result<Option<TokenUsage>, String> {
        let Some(usage) = payload.get("usage").filter(|usage| !usage.is_null()) else {
            return Ok(None);
        };
        let field = |name: &str| -> Result<u32, String> {
            match usage.get(name) {
                None | Some(Value::Null) => Ok(0),
                Some(value) => {
                    let count = value.as_u64().ok_or_else(|| {
                        format!(
                            "`usage.{name}` must be a non-negative integer, got {}",
                            json_type_name(value)
                        )
                    })?;
                    u32::try_from(count).map_err(|_| {
                        format!("`usage.{name}` is {count}, beyond the range a token count carries")
                    })
                }
            }
        };
        let prompt_tokens = field("prompt_tokens")?;
        let completion_tokens = field("completion_tokens")?;
        if prompt_tokens == 0 && completion_tokens == 0 {
            return Ok(None);
        }
        Ok(Some(TokenUsage {
            prompt_tokens,
            completion_tokens,
        }))
    }

    /// The choice's `finish_reason`, or why it cannot be read.
    ///
    /// Absent, null and empty all mean "not reported yet" — a streamed frame
    /// carries one only at the end. A present non-string does not: `as_str`
    /// used to turn it into that same absence, and `guest-openai` renders an
    /// absent reason as `stop`, so an upstream that truncated at its own token
    /// limit could report the answer complete. A client then runs half a
    /// function believing it read the whole one.
    fn read_finish_reason(choice: Option<&Value>) -> Result<Option<String>, String> {
        match choice.and_then(|choice| choice.get("finish_reason")) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(reason)) if reason.is_empty() => Ok(None),
            Some(Value::String(reason)) => Ok(Some(reason.clone())),
            Some(other) => Err(format!(
                "`choices[0].finish_reason` must be a string, got {}",
                json_type_name(other)
            )),
        }
    }

    pub(crate) fn generate(&self, prompts: &[&[u8]]) -> Result<UpstreamGeneration, UpstreamError> {
        let [prompt] = prompts else {
            return Err(UpstreamError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!(
                    "upstream generation takes exactly one prompt, got {}",
                    prompts.len()
                ),
            });
        };
        let (body, timeout) = self.chat_body(prompt, false)?;
        let response = self.post("/chat/completions", &body, timeout)?;
        let payload: Value = read_json(&self.alias, response)?;
        // Every OpenAI-shaped upstream returns `usage` on the buffered route,
        // so unlike the streaming path this is reliably populated.
        let usage =
            Self::read_usage(&payload).map_err(|detail| UpstreamError::MalformedResponse {
                alias: self.alias.clone(),
                detail,
            })?;
        // `length` means the upstream hit its own token limit, so the answer is
        // truncated. Reporting it as `stop` let a client run half a function.
        let finish_reason = Self::read_finish_reason(
            payload
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first()),
        )
        .map_err(|detail| UpstreamError::MalformedResponse {
            alias: self.alias.clone(),
            detail,
        })?;
        let message = payload
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .ok_or_else(|| UpstreamError::MalformedResponse {
                alias: self.alias.clone(),
                detail: "response has no `choices[0].message` object".to_owned(),
            })?;
        // Absent and `null` both mean "this message carried no text", which is
        // the normal shape beside `tool_calls`. A *present* non-string — an
        // object, an array — is neither: reading it with `as_str` alone
        // reported it as absent, so the tool-call branch below returned empty
        // bytes while reporting a successful structured call, and the caller
        // had no way to know an assistant message had been discarded. The
        // streamed path draws exactly this line on `delta.content`; the
        // buffered one has to draw it too, or the same response is rejected or
        // silently emptied depending on which path served it.
        let content = match message.get("content") {
            None | Some(Value::Null) => None,
            Some(value) => {
                Some(
                    value
                        .as_str()
                        .ok_or_else(|| UpstreamError::MalformedResponse {
                            alias: self.alias.clone(),
                            detail: format!(
                                "response has a non-string `choices[0].message.content` of type {}",
                                json_type_name(value)
                            ),
                        })?,
                )
            }
        };

        // A tool call carries `content: null`, so requiring a content string
        // would turn every successful tool call into a malformed-response
        // error. The calls travel on `tool_calls`, structured, rather than
        // re-encoded into the text: the accelerator interface has a channel for
        // them, so the caller never has to recognise a smuggled envelope and
        // never mistakes a model answering in plain JSON for one.
        if let Some(tool_calls) = message
            .get("tool_calls")
            .filter(|tool_calls| !is_empty_json(tool_calls))
        {
            return Ok(UpstreamGeneration {
                bytes: content.unwrap_or_default().as_bytes().to_vec(),
                usage,
                finish_reason,
                tool_calls: tool_calls_from_value(&self.alias, tool_calls)?,
            });
        }

        // The shape OpenAI-compatible providers used before `tool_calls`, and
        // still the only one some of them emit. It carries a single call with
        // no id. Recognising it costs one branch; not recognising it made every
        // buffered request to such a provider fail as malformed, since neither
        // content nor `tool_calls` is present.
        if let Some(legacy) = message
            .get("function_call")
            .filter(|call| !is_empty_json(call))
        {
            let calls = tool_calls_from_value(
                &self.alias,
                &Value::Array(vec![json!({
                    "type": "function",
                    "function": legacy,
                })]),
            )?;
            return Ok(UpstreamGeneration {
                bytes: content.unwrap_or_default().as_bytes().to_vec(),
                usage,
                finish_reason,
                tool_calls: calls,
            });
        }

        // A reported finish reason makes an absent content an *answer* — an
        // empty one. A safety filter returning `content_filter` with
        // `content: null` is the common case, and rejecting it handed the
        // client a 500 in place of the result the upstream actually reached.
        // Only a response with no content, no calls and no reason is shapeless
        // enough to have nothing to report.
        let Some(text) = content else {
            if finish_reason.is_some() {
                return Ok(UpstreamGeneration {
                    bytes: Vec::new(),
                    usage,
                    finish_reason,
                    tool_calls: Vec::new(),
                });
            }
            return Err(UpstreamError::MalformedResponse {
                alias: self.alias.clone(),
                detail:
                    "response has no `choices[0].message.content` string, no `tool_calls` and no \
                     `finish_reason`"
                        .to_owned(),
            });
        };
        Ok(UpstreamGeneration {
            bytes: text.as_bytes().to_vec(),
            usage,
            finish_reason,
            tool_calls: Vec::new(),
        })
    }

    /// Stream one generation, invoking `sink` per SSE delta so the mesh's own
    /// `/ai/v1` stream keeps a real time-to-first-token.
    pub(crate) fn generate_streaming(
        &self,
        prompts: &[&[u8]],
        sink: &mut dyn StreamSink,
    ) -> Result<StreamOutcome, UpstreamError> {
        let [prompt] = prompts else {
            return Err(UpstreamError::InvalidRequest {
                alias: self.alias.clone(),
                detail: format!(
                    "upstream streaming takes exactly one prompt, got {}",
                    prompts.len()
                ),
            });
        };
        let (body, timeout) = self.chat_body(prompt, true)?;
        // Content is streamed unconditionally, including when the request
        // offered tools. That is what the structured `StreamEvent::ToolCall`
        // arm buys: with calls confined to the text channel, prose had to be
        // accumulated until the stream ended, because a tool-call envelope
        // emitted after streamed prose is unparseable — so offering tools cost
        // the whole time-to-first-token even on the requests that never
        // produced a call.
        let response = self.post("/chat/completions", &body, timeout)?;

        // Bound the whole stream, so an upstream that never terminates cannot
        // grow the reader without limit.
        let reader = std::io::BufReader::new(std::io::Read::take(response, MAX_STREAM_BYTES));
        // Read on its own thread so this loop can answer for itself while the
        // upstream says nothing.
        //
        // `read_line` is uninterruptible, and a stream's cancellation signal
        // only ever arrived through `sink`'s return value — which needs a frame
        // to have landed first. A client that disconnected while the upstream
        // was still thinking therefore went unnoticed until the request
        // timeout, up to an hour, with the admission permit held the whole
        // time; thirty-two of those exhaust the node's upstream capacity.
        // reqwest's blocking client has no per-read timeout to lean on, so the
        // blocking read moves off this thread and the poll below asks
        // `is_live()` between frames instead.
        //
        // The reader is bounded by the admission permit that is already held
        // here, so this adds no thread a burst could multiply. It ends when the
        // receiver drops: the send fails, the response is dropped, the socket
        // closes.
        let mut lines = SseLineReader::spawn(reader);
        let mut line = String::new();
        let mut saw_done = false;
        let mut usage = None;
        let mut finish_reason = None;
        // Set when the sink asks to stop — the client went away. The read loop
        // then abandons the upstream response instead of draining it to
        // `[DONE]`, which is what releases the socket, the thread and the
        // admission permit for a live request.
        let mut abandoned = false;
        let mut streamed_tool_calls = StreamedToolCalls::default();
        loop {
            line.clear();
            let next = loop {
                match lines.next_line(SSE_LIVENESS_POLL) {
                    SseRead::Line(line) => break Some(line),
                    SseRead::Eof => break None,
                    SseRead::Failed(detail) => {
                        return Err(UpstreamError::Transport {
                            alias: self.alias.clone(),
                            endpoint: self.endpoint.url("/chat/completions"),
                            detail,
                        })
                    }
                    // Nothing yet. The only question worth asking while an
                    // upstream is silent is whether anyone is still waiting for
                    // it.
                    SseRead::Idle => {
                        if !sink.is_live() {
                            abandoned = true;
                            break None;
                        }
                    }
                }
            };
            let Some(next) = next else {
                break;
            };
            line.push_str(&next);
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
            // A malformed data frame is not skippable: if it was carrying a
            // content delta, skipping it truncates the answer and `[DONE]` then
            // reports the whole request as successful.
            let frame = serde_json::from_str::<Value>(payload).map_err(|error| {
                UpstreamError::MalformedResponse {
                    alias: self.alias.clone(),
                    detail: format!("upstream sent an SSE data frame that is not JSON: {error}"),
                }
            })?;
            // Valid JSON is not yet a frame. A scalar or an array parses, and
            // then every `.get(...)` below answers `None` — so `data: "answer"`
            // read as a usage-only event, its text vanished, and a following
            // `[DONE]` completed the request successfully.
            if !frame.is_object() {
                return Err(UpstreamError::MalformedResponse {
                    alias: self.alias.clone(),
                    detail: format!(
                        "upstream SSE data frame must be a JSON object, got {}",
                        json_type_name(&frame)
                    ),
                });
            }
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
            // Asked once per frame, before anything is emitted. `emit` only
            // reports a departed consumer when there is something to emit, so a
            // stream of role-only openings, usage frames or keep-alives would
            // otherwise run to completion for a client that had already gone —
            // holding a socket, a thread and an admission permit throughout.
            if !sink.is_live() {
                abandoned = true;
                break;
            }
            if let Some(reported) =
                Self::read_usage(&frame).map_err(|detail| UpstreamError::MalformedResponse {
                    alias: self.alias.clone(),
                    detail,
                })?
            {
                usage = Some(reported);
            }
            // Absent is a usage-only frame and an empty array is a legal
            // keep-alive, so neither is an error. A *present* non-array is:
            // `as_array` alone turned `{"choices":{"delta":{"content":"…"}}}`
            // into no choice at all, so the frame's content was dropped and a
            // following `[DONE]` still completed the stream successfully with
            // the guest reporting an ordinary `stop`. Silently losing part of
            // an answer is worse than failing the request.
            let choices = match frame.get("choices") {
                None | Some(Value::Null) => None,
                Some(value) => {
                    Some(
                        value
                            .as_array()
                            .ok_or_else(|| UpstreamError::MalformedResponse {
                                alias: self.alias.clone(),
                                detail: format!(
                                    "streamed frame has a non-array `choices` of type {}",
                                    json_type_name(value)
                                ),
                            })?,
                    )
                }
            };
            // Same rule one level in: an empty array is a legal keep-alive, but
            // a present entry that is not an object is not a choice. Kept, it
            // read as one that merely omitted `delta` and `finish_reason`, so
            // `{"choices":[42]}` was discarded whole and `[DONE]` still
            // completed the stream on an ordinary `stop`.
            let choice = match choices.and_then(|choices| choices.first()) {
                None => None,
                Some(choice) if choice.is_object() => Some(choice),
                Some(other) => {
                    return Err(UpstreamError::MalformedResponse {
                        alias: self.alias.clone(),
                        detail: format!(
                            "streamed `choices[0]` must be an object, got {}",
                            json_type_name(other)
                        ),
                    })
                }
            };
            // Sent on the last content-bearing frame, before `[DONE]`. Keeping
            // it is what lets the caller tell a completion that finished from
            // one the upstream truncated at its own token limit.
            if let Some(reported) = Self::read_finish_reason(choice).map_err(|detail| {
                UpstreamError::MalformedResponse {
                    alias: self.alias.clone(),
                    detail,
                }
            })? {
                // Two different reasons for one choice is not a stream this
                // side can reconcile. Last-write-wins let a `length` — the
                // answer was truncated — be overwritten by a later `stop`,
                // which is precisely the direction that loses information the
                // caller acts on.
                if finish_reason
                    .as_deref()
                    .is_some_and(|earlier| earlier != reported)
                {
                    return Err(UpstreamError::MalformedResponse {
                        alias: self.alias.clone(),
                        detail: format!(
                            "upstream reported conflicting finish reasons for one choice: \
                             `{}` then `{reported}`",
                            finish_reason.unwrap_or_default()
                        ),
                    });
                }
                finish_reason = Some(reported);
            }
            // The same rule one level up: a frame legitimately carries no
            // `delta` — a final frame that only reports `finish_reason` does —
            // but a *present* non-object is not that. Accepting it left every
            // `delta.get(...)` below reading it as neither content nor tool
            // call, so `{"choices":[{"delta":"answer"}]}` was discarded whole
            // and `[DONE]` still completed the stream on an ordinary `stop`.
            let delta = match choice.and_then(|choice| choice.get("delta")) {
                None | Some(Value::Null) => continue,
                Some(delta) if delta.is_object() => delta,
                Some(delta) => {
                    return Err(UpstreamError::MalformedResponse {
                        alias: self.alias.clone(),
                        detail: format!(
                            "streamed frame has a non-object `delta` of type {}",
                            json_type_name(delta)
                        ),
                    });
                }
            };
            // Absent and `null` both mean "this frame carried no text" — the
            // normal shape of a tool-call or role-only frame. A *present*
            // non-string is different: reading it with `as_str` alone reported
            // it as absent, so that part of the answer was dropped and the
            // stream still finished successfully. The buffered path rejects an
            // unusable message; this one now matches it, rather than returning
            // a silently short answer a caller cannot detect.
            match delta.get("content") {
                None | Some(Value::Null) => {}
                Some(Value::String(content)) => {
                    if !content.is_empty() && sink.emit(StreamEvent::Content(content)).is_stop() {
                        abandoned = true;
                        break;
                    }
                }
                Some(other) => {
                    return Err(UpstreamError::MalformedResponse {
                        alias: self.alias.clone(),
                        detail: format!(
                            "streamed `choices[0].delta.content` is {}, expected a string",
                            json_type_name(other)
                        ),
                    })
                }
            }
            // A streamed tool call arrives as `delta.tool_calls` fragments with
            // no content at all. Dropping them would make the whole request
            // look like a model that answered with silence, so accumulate them
            // and emit each assembled call once the stream ends — a call is
            // only dispatchable complete, and its arguments arrive in pieces.
            if let Some(fragments) = delta
                .get("tool_calls")
                .filter(|value| !is_empty_json(value))
            {
                // Validated rather than skipped, matching the buffered path. A
                // non-array here used to read as "no tool calls at all", so a
                // stream could finish successfully — with the upstream's own
                // `finish_reason: "tool_calls"` — while emitting nothing, and
                // the client would be told to dispatch a call it never got.
                let fragments =
                    fragments
                        .as_array()
                        .ok_or_else(|| UpstreamError::MalformedResponse {
                            alias: self.alias.clone(),
                            detail: format!(
                                "`choices[0].delta.tool_calls` must be an array, got {}",
                                json_type_name(fragments)
                            ),
                        })?;
                for fragment in fragments {
                    streamed_tool_calls.absorb(fragment).map_err(|detail| {
                        UpstreamError::MalformedResponse {
                            alias: self.alias.clone(),
                            detail,
                        }
                    })?;
                }
            }
        }

        // Emitted only once the stream is known to have finished. A tool call
        // is an instruction the caller acts on, and this used to hand them over
        // before the missing-sentinel check below could refuse the stream — so
        // an upstream that died mid-generation, after its call fragments but
        // before `[DONE]`, got its half-finished intent dispatched and *then*
        // reported the request as failed. The caller had already run it.
        //
        // `abandoned` keeps its own exemption: nobody is left to receive the
        // calls, so there is nothing to be premature about.
        if !abandoned && saw_done {
            for call in
                streamed_tool_calls
                    .finish()
                    .map_err(|detail| UpstreamError::MalformedResponse {
                        alias: self.alias.clone(),
                        detail,
                    })?
            {
                if sink.emit(StreamEvent::ToolCall(call)).is_stop() {
                    abandoned = true;
                    break;
                }
            }
        }

        // A clean EOF without `[DONE]` is a truncated generation, not a
        // completed one — the upstream restarted mid-stream, or returned a
        // non-SSE success body. Reporting success here would hand the caller
        // silently truncated code and record the request as healthy.
        //
        // An abandoned stream is exempt: nobody is left to receive the answer,
        // so stopping short is the intended outcome rather than a truncation to
        // report.
        if !saw_done && !abandoned {
            return Err(UpstreamError::MalformedResponse {
                alias: self.alias.clone(),
                detail: "upstream stream ended before the `[DONE]` sentinel".to_owned(),
            });
        }
        // Absence stays absence: an upstream that volunteers no usage frame
        // has told us nothing, and zeros would read to the client as a
        // generation that cost nothing.
        Ok(StreamOutcome {
            usage,
            finish_reason,
        })
    }

    /// Forward a single embedding request to the upstream `/embeddings` route.
    pub(crate) fn embed(&self, input: &str) -> Result<Vec<f32>, UpstreamError> {
        let body = json!({"model": self.endpoint.model, "input": input});
        let response = self.post("/embeddings", &body, self.endpoint.timeout)?;
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
        let mut vector = embedding
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
            .collect::<Result<Vec<f32>, UpstreamError>>()?;

        // The `embed` contract promises an L2-normalized vector, and the local
        // embedding backend delivers one — so a consumer treating a dot product
        // as cosine similarity is reading the contract correctly. An upstream
        // that normalizes is common but not universal, and returning a raw
        // vector made rankings magnitude-dependent *only* for upstream-bound
        // aliases: the same query scored differently depending on which backend
        // happened to serve the alias, with nothing in the response saying so.
        //
        // Normalizing here rather than trusting the server also makes the
        // double-normalization case free: a unit vector is its own normal.
        normalize_in_place(&mut vector).map_err(|detail| UpstreamError::MalformedResponse {
            alias: self.alias.clone(),
            detail,
        })?;
        Ok(vector)
    }
}

/// Scale a vector to unit length in place.
///
/// The norm is accumulated in `f64`: a 4096-dimension embedding with components
/// near `f32::MAX.sqrt()` overflows an `f32` accumulator to infinity, which
/// would turn a perfectly good vector into NaNs.
///
/// A zero-norm vector is rejected rather than passed through. It cannot be
/// normalized at all, and every cosine similarity against it is undefined — so
/// returning it would satisfy the call and break the contract the caller is
/// relying on.
fn normalize_in_place(vector: &mut [f32]) -> Result<(), String> {
    let norm = vector
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt();
    if !norm.is_finite() || norm == 0.0 {
        return Err(format!(
            "`data[0].embedding` has an L2 norm of {norm}, so it cannot be normalized to the unit \
             length the `embed` contract promises"
        ));
    }
    for value in vector.iter_mut() {
        *value = (f64::from(*value) / norm) as f32;
    }
    Ok(())
}

/// `true` for JSON that carries nothing worth forwarding: absent, null, or an
/// empty array. Lets an explicitly-empty `tools: []` be dropped rather than
/// sent, which some upstreams reject.
/// Reject a `tool_calls` payload the downstream parser could not use.
///
/// Checked here rather than in the guest because this is where the upstream's
/// response is still typed: by the time it reaches `guest-openai` it is text,
/// and an unusable call is indistinguishable from a model that answered in
/// JSON.
/// Validate and convert an upstream `message.tool_calls` array.
///
/// Forwarding an unusable array is worse than failing: the caller cannot
/// dispatch a call with no function name, and a mixed array is the same problem
/// one entry at a time — the invalid calls would simply vanish while the
/// response still reported success.
fn tool_calls_from_value(alias: &str, tool_calls: &Value) -> Result<Vec<ToolCall>, UpstreamError> {
    let malformed = |detail: String| UpstreamError::MalformedResponse {
        alias: alias.to_owned(),
        detail,
    };
    let calls = tool_calls.as_array().ok_or_else(|| {
        malformed(format!(
            "`choices[0].message.tool_calls` must be an array, got {}",
            json_type_name(tool_calls)
        ))
    })?;
    calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            let function = call.get("function");
            let name = function
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if name.trim().is_empty() {
                return Err(malformed(format!(
                    "`choices[0].message.tool_calls[{index}]` has no `function.name`"
                )));
            }
            // The guest hardcodes `"function"` back onto whatever it echoes, so
            // an upstream naming a different kind here would have that kind
            // silently relabelled and dispatched as a function call. Absent is
            // fine — the field is optional and `function` is its only defined
            // value — but a present, different one is a call this backend does
            // not know how to carry.
            if let Some(kind) = call.get("type").and_then(Value::as_str) {
                if kind != "function" {
                    return Err(malformed(format!(
                        "`choices[0].message.tool_calls[{index}].type` is `{kind}`, and only \
                         `function` calls can be carried"
                    )));
                }
            }
            let arguments = tool_call_arguments(function.and_then(|f| f.get("arguments")))
                .ok_or_else(|| {
                    malformed(format!(
                        "`choices[0].message.tool_calls[{index}].function.arguments` is neither a JSON string nor a JSON object"
                    ))
                })?;
            Ok(ToolCall {
                id: call
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_owned),
                name: name.to_owned(),
                arguments: arguments.to_owned(),
            })
        })
        .collect()
}

/// The argument JSON for one tool call, whatever shape the upstream used.
///
/// OpenAI specifies `function.arguments` as a *string* of JSON, and most
/// servers comply — but several emit the object itself. Reading only the string
/// form treated a present object as absent, and the caller then dispatched the
/// correct function with `{}`: the right tool, none of its arguments, and a
/// generation reported as successful. An object is serialized rather than
/// dropped; `None` means the value was neither, which the caller reports as
/// malformed instead of inventing an empty argument set.
fn tool_call_arguments(arguments: Option<&Value>) -> Option<String> {
    match arguments {
        // Absent or explicitly null: a function that takes no arguments.
        None | Some(Value::Null) => Some("{}".to_owned()),
        Some(Value::String(raw)) if raw.trim().is_empty() => Some("{}".to_owned()),
        Some(Value::String(raw)) => Some(raw.clone()),
        Some(value @ (Value::Object(_) | Value::Array(_))) => Some(value.to_string()),
        // A bare number or boolean is not an argument set under either reading.
        Some(_) => None,
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

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
/// One call being reassembled. A named struct rather than a tuple because it
/// now carries a validity flag as well as the three accumulating fields.
#[derive(Default)]
struct StreamedToolCall {
    index: u64,
    id: String,
    name: String,
    /// Concatenated argument text, or one whole object's rendering.
    arguments: String,
    /// Set when a fragment's `arguments` was neither a string nor an object.
    /// Reported by `finish` rather than silently reduced to `{}`.
    unusable_arguments: bool,
}

#[derive(Default)]
struct StreamedToolCalls {
    /// In first-seen order, so the emitted list preserves the upstream's own
    /// ordering.
    calls: Vec<StreamedToolCall>,
}

impl StreamedToolCalls {
    /// Absorb one `delta.tool_calls` fragment, or fail if it cannot be
    /// attributed to a call.
    ///
    /// `index` is what joins fragments into calls, and it was defaulted to 0
    /// when missing or oddly typed. That silently *merged* distinct calls into
    /// one slot — the later id and name overwrote the earlier, while arguments
    /// concatenated across both — so the caller could dispatch one function
    /// with another's arguments. There is no safe default here: guessing which
    /// call a fragment belongs to is exactly the mistake.
    ///
    /// Deliberately strict. OpenAI's streaming schema requires `index` on every
    /// tool-call delta, so an upstream omitting it is not one this backend can
    /// reassemble correctly, and failing says so instead of inventing an
    /// answer.
    fn absorb(&mut self, fragment: &Value) -> Result<(), String> {
        let index = match fragment.get("index") {
            Some(value) => value.as_u64().ok_or_else(|| {
                format!(
                    "streamed tool-call fragment has `index` of {}, expected a non-negative integer",
                    json_type_name(value)
                )
            })?,
            None => {
                return Err(
                    "streamed tool-call fragment has no `index`, so it cannot be attributed to a call"
                        .to_owned(),
                )
            }
        };
        let slot = match self.calls.iter_mut().find(|call| call.index == index) {
            Some(slot) => slot,
            None => {
                self.calls.push(StreamedToolCall {
                    index,
                    ..StreamedToolCall::default()
                });
                self.calls.last_mut().expect("just pushed")
            }
        };
        // An id or a name is sent once, on the opening fragment. A *second*,
        // different one for the same index is not an update — it means two
        // calls are being merged under one slot, and every argument fragment
        // seen so far belongs to whichever one this is not. Overwriting kept
        // the accumulated arguments and swapped the identity on top of them, so
        // the caller dispatched a call that never existed: one function's name
        // with another's arguments, echoed back under an id the upstream never
        // paired with them.
        if let Some(id) = fragment
            .get("id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            if !slot.id.is_empty() && slot.id != id {
                return Err(format!(
                    "streamed tool call at index {index} was given a second id: `{}` then `{id}`",
                    slot.id
                ));
            }
            slot.id = id.to_owned();
        }
        let function = fragment.get("function");
        if let Some(name) = function
            .and_then(|function| function.get("name"))
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        {
            if !slot.name.is_empty() && slot.name != name {
                return Err(format!(
                    "streamed tool call at index {index} was given a second function name: \
                     `{}` then `{name}`",
                    slot.name
                ));
            }
            slot.name = name.to_owned();
        }
        // A streamed call's arguments arrive as string slices that concatenate,
        // so a string is appended verbatim. An upstream that sends the object
        // itself is not streaming it in pieces — that value *replaces* what was
        // accumulated, because appending an object's rendering to a partial
        // string would corrupt both. Reading only the string form dropped the
        // object entirely and dispatched the call with no arguments at all.
        match function.and_then(|function| function.get("arguments")) {
            Some(Value::String(raw)) => slot.arguments.push_str(raw),
            Some(value @ (Value::Object(_) | Value::Array(_))) => {
                slot.arguments = value.to_string();
            }
            // Absent on most fragments, and on the opening one that carries
            // only the id and the name.
            None | Some(Value::Null) => {}
            Some(_) => slot.unusable_arguments = true,
        }
        Ok(())
    }

    /// Assemble the accumulated fragments, or fail if any of them never named
    /// its function or carried arguments in an unusable shape.
    ///
    /// Dropping an unnamed call silently was the wrong shape of forgiveness:
    /// the caller received an empty or short tool-call set while telemetry
    /// recorded a healthy generation, so an agent simply saw the model decline
    /// to act. Once fragments have been observed the upstream has committed to
    /// a call, and an incomplete one means the stream was truncated.
    fn finish(self) -> Result<Vec<ToolCall>, String> {
        if let Some(call) = self.calls.iter().find(|call| call.name.trim().is_empty()) {
            return Err(format!(
                "upstream streamed tool-call fragments for index {} but never sent a function name",
                call.index
            ));
        }
        // Same rule as the buffered path: an argument value that is neither a
        // string nor an object cannot be reduced to `{}` without handing the
        // caller the right function and the wrong arguments.
        if let Some(call) = self.calls.iter().find(|call| call.unusable_arguments) {
            return Err(format!(
                "upstream streamed `function.arguments` for index {} that is neither a JSON string nor a JSON object",
                call.index
            ));
        }
        // Concatenating string fragments is how a streamed call's arguments
        // arrive, so a stream that ends mid-value leaves something like
        // `{"path":` behind — syntactically a string, semantically half a call.
        // Passed through, it reaches the client as a successful, dispatchable
        // invocation whose arguments cannot be parsed; whether that surfaces as
        // a crash or as a silently skipped tool depends on the client. The
        // assembled text has to be a JSON object, which is what the OpenAI
        // schema promises and what every caller assumes.
        for call in &self.calls {
            if call.arguments.is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(&call.arguments) {
                Ok(Value::Object(_)) => {}
                Ok(other) => {
                    return Err(format!(
                        "upstream streamed `function.arguments` for index {} that is a JSON {}, \
                         not an object",
                        call.index,
                        json_type_name(&other)
                    ))
                }
                Err(error) => {
                    return Err(format!(
                        "upstream streamed `function.arguments` for index {} that is not valid \
                         JSON: {error}",
                        call.index
                    ))
                }
            }
        }
        Ok(self
            .calls
            .into_iter()
            .map(|call| ToolCall {
                id: (!call.id.is_empty()).then_some(call.id),
                name: call.name,
                // An empty argument string is not valid JSON, and a caller that
                // parses it would report a broken call for a function that
                // simply takes none.
                arguments: if call.arguments.is_empty() {
                    "{}".to_owned()
                } else {
                    call.arguments
                },
            })
            .collect())
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

    /// A stream split into the two things the contract now separates: assistant
    /// text, and structured calls. Assertions on the pair are what prove the
    /// separation holds — that content is not withheld waiting for a call, and
    /// that a call is not re-encoded into the text.
    #[derive(Default)]
    struct CapturedStream {
        content: Vec<String>,
        tool_calls: Vec<ToolCall>,
    }

    impl CapturedStream {
        fn sink(&mut self) -> impl FnMut(StreamEvent<'_>) -> StreamControl + '_ {
            move |event| {
                match event {
                    StreamEvent::Content(text) => self.content.push(text.to_owned()),
                    StreamEvent::ToolCall(call) => self.tool_calls.push(call),
                }
                StreamControl::Continue
            }
        }

        /// A sink that walks away after `after` content events, the way a
        /// disconnected SSE client does.
        fn sink_stopping_after(
            &mut self,
            after: usize,
        ) -> impl FnMut(StreamEvent<'_>) -> StreamControl + '_ {
            move |event| {
                match event {
                    StreamEvent::Content(text) => self.content.push(text.to_owned()),
                    StreamEvent::ToolCall(call) => self.tool_calls.push(call),
                }
                if self.content.len() >= after {
                    StreamControl::Stop
                } else {
                    StreamControl::Continue
                }
            }
        }
    }

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
        let (body, _timeout) = backend
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

        let (body, _timeout) = backend
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

        let (body, _timeout) = backend
            .chat_body(br#"{"prompt":"p"}"#, false)
            .expect("request should map");
        assert_eq!(body["max_tokens"], 6000);

        let (body, _timeout) = backend
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
        let (body, _timeout) = backend
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
        let (body, _timeout) = backend
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
        let (body, _timeout) = backend
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

        let generation = backend
            .generate(&[br#"{"messages":[{"role":"user","content":"write main"}]}"#])
            .expect("generation should round trip");
        assert_eq!(generation.bytes, b"fn main() {}".to_vec());
        assert!(generation.tool_calls.is_empty());

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
                // An empty delta — the usual role-only opening frame — must not
                // emit a token.
                "data: {\"choices\":[{\"delta\":{\"content\":\"\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"()\"}}]}\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut captured = CapturedStream::default();
        backend
            .generate_streaming(&[b"write main"], &mut captured.sink())
            .expect("streaming should complete");
        assert_eq!(captured.content, vec!["fn ", "main", "()"]);
        assert!(captured.tool_calls.is_empty());

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
        // Normalized on the way out: the `embed` contract promises unit length,
        // and the raw `[0.25, -0.5, 1.0]` has a norm of ~1.1456.
        let norm = (0.25f32 * 0.25 + 0.5 * 0.5 + 1.0f32).sqrt();
        let expected = [0.25 / norm, -0.5 / norm, 1.0 / norm];
        assert_eq!(vector.len(), expected.len());
        for (got, want) in vector.iter().zip(expected) {
            assert!((got - want).abs() < 1e-6, "got {got}, want {want}");
        }

        let (target, body) = upstream.received();
        assert!(
            target.starts_with("POST /v1/embeddings "),
            "unexpected request line: {target}"
        );
        assert_eq!(body["input"], "hello");
    }

    /// A silent source must report `Idle`, not hold the caller.
    ///
    /// This is the seam the cancellation fix turns on: `read_line` is
    /// uninterruptible, so the read moved to its own thread and the stream loop
    /// polls `is_live` between frames. Exercised directly rather than through a
    /// real socket — a round trip adds a connect that can fail under load for
    /// reasons that have nothing to do with the mechanism, and a test that
    /// fails for the wrong reason is worse than no test. The end-to-end
    /// behaviour is covered by
    /// `a_stream_carrying_no_content_still_notices_a_departed_client`.
    #[test]
    fn a_reader_with_nothing_to_give_reports_idle_rather_than_blocking() {
        /// Blocks in `read` until the test feeds it, the way a socket blocks on
        /// a silent upstream.
        struct Fed(std::sync::mpsc::Receiver<Vec<u8>>);
        impl std::io::Read for Fed {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                match self.0.recv() {
                    Ok(chunk) => {
                        let len = chunk.len().min(buf.len());
                        buf[..len].copy_from_slice(&chunk[..len]);
                        Ok(len)
                    }
                    // The far end went away: EOF, as a closed socket reads.
                    Err(_) => Ok(0),
                }
            }
        }

        let (feed, fed) = std::sync::mpsc::channel();
        let mut reader = SseLineReader::spawn(std::io::BufReader::new(Fed(fed)));

        let started = std::time::Instant::now();
        assert!(
            matches!(reader.next_line(Duration::from_millis(50)), SseRead::Idle),
            "a source with nothing to give must yield the caller, not hold it"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the poll must return on its own window; took {:?}",
            started.elapsed()
        );

        feed.send(b"data: hi\n".to_vec()).expect("fixture feed");
        let line = loop {
            match reader.next_line(Duration::from_millis(250)) {
                SseRead::Line(line) => break line,
                SseRead::Idle => continue,
                other => panic!("expected the fed line, got {}", sse_read_name(&other)),
            }
        };
        assert_eq!(
            line, "data: hi\n",
            "a fed line arrives intact after the idle poll"
        );

        drop(feed);
        loop {
            match reader.next_line(Duration::from_millis(250)) {
                SseRead::Eof => break,
                SseRead::Idle => continue,
                other => panic!(
                    "a closed source must read as EOF, got {}",
                    sse_read_name(&other)
                ),
            }
        }
    }

    fn sse_read_name(read: &SseRead) -> &'static str {
        match read {
            SseRead::Line(_) => "a line",
            SseRead::Idle => "idle",
            SseRead::Eof => "eof",
            SseRead::Failed(_) => "a failure",
        }
    }

    /// A gateway answers 502 when its upstream does not answer usably.
    ///
    /// `None` renders as 500 `server_error` — this node broke — and an agent
    /// retrying that against a healthy node retries forever against an upstream
    /// that is not. The two genuinely local faults keep `None`, because neither
    /// ever reached a provider.
    #[test]
    fn an_upstream_that_cannot_answer_is_reported_as_a_gateway_failure() {
        let status = |error: UpstreamError| error.http_status();

        assert_eq!(
            status(UpstreamError::Transport {
                alias: "coder".to_owned(),
                endpoint: "http://gone.invalid/v1".to_owned(),
                detail: "connection refused".to_owned(),
            }),
            Some(502)
        );
        assert_eq!(
            status(UpstreamError::MalformedResponse {
                alias: "coder".to_owned(),
                detail: "not JSON".to_owned(),
            }),
            Some(502)
        );
        // A remote status is specific and actionable; flattening it loses that.
        assert_eq!(
            status(UpstreamError::Status {
                alias: "coder".to_owned(),
                endpoint: "http://up.invalid/v1".to_owned(),
                status: 429,
                body: "slow down".to_owned(),
            }),
            Some(429)
        );
        assert_eq!(
            status(UpstreamError::InvalidRequest {
                alias: "coder".to_owned(),
                detail: "no prompt".to_owned(),
            }),
            None,
            "a request that never left this node is not the provider's failure"
        );
        assert_eq!(
            status(UpstreamError::InvalidBinding {
                alias: "coder".to_owned(),
                detail: "bad url".to_owned(),
            }),
            None,
            "a malformed binding is this deployment's own configuration"
        );
    }

    /// The pre-`tool_calls` shape some providers still emit.
    ///
    /// It carries neither content nor `tool_calls`, so every buffered request
    /// to such a provider used to fail as malformed.
    #[test]
    fn a_legacy_function_call_response_is_carried_as_a_tool_call() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"choices":[{"message":{"content":null,
                "function_call":{"name":"read_file","arguments":"{\"path\":\"a.rs\"}"}},
                "finish_reason":"function_call"}]}"#,
        );
        let backend = runtime("coder", &upstream.binding());
        let generation = backend.generate(&[b"go"]).expect("legacy call round trip");
        assert_eq!(generation.tool_calls.len(), 1);
        assert_eq!(generation.tool_calls[0].name, "read_file");
        assert_eq!(generation.tool_calls[0].arguments, r#"{"path":"a.rs"}"#);
        assert!(
            generation.tool_calls[0].id.is_none(),
            "the legacy shape carries no id, and one must not be invented"
        );
    }

    /// A filtered completion is a result, not a malformed response.
    #[test]
    fn a_content_filtered_completion_without_content_is_an_empty_answer() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"choices":[{"message":{"content":null},"finish_reason":"content_filter"}]}"#,
        );
        let backend = runtime("coder", &upstream.binding());
        let generation = backend
            .generate(&[b"go"])
            .expect("a filtered completion is a result the client must see");
        assert!(generation.bytes.is_empty());
        assert_eq!(generation.finish_reason.as_deref(), Some("content_filter"));
    }

    /// With no content, no calls and no reason there is nothing to report, and
    /// that stays an error.
    #[test]
    fn a_message_with_nothing_at_all_is_still_malformed() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"choices":[{"message":{"content":null}}]}"#,
        );
        let backend = runtime("coder", &upstream.binding());
        let Err(error) = backend.generate(&[b"go"]) else {
            panic!("a message with no content, no calls and no reason reports nothing");
        };
        assert!(error.to_string().contains("no `finish_reason`"), "{error}");
    }

    /// The guest relabels whatever it echoes as `function`, so a different kind
    /// would be dispatched as one.
    #[test]
    fn a_tool_call_of_another_type_is_refused_rather_than_relabelled() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"choices":[{"message":{"content":null,"tool_calls":[
                {"id":"c1","type":"custom","function":{"name":"f","arguments":"{}"}}]}}]}"#,
        );
        let backend = runtime("coder", &upstream.binding());
        let Err(error) = backend.generate(&[b"go"]) else {
            panic!("a non-function call type must not be silently relabelled");
        };
        assert!(error.to_string().contains("only `function`"), "{error}");
    }

    /// A call is an instruction, so it waits for the stream to finish.
    ///
    /// An upstream that dies after its tool-call fragments but before `[DONE]`
    /// used to have its half-finished intent dispatched, and *then* the request
    /// reported as failed — by which time the caller had already run it. The
    /// stream still fails; what changes is that nothing was handed over first.
    #[test]
    fn a_tool_call_is_not_emitted_by_a_stream_that_never_finished() {
        let emitted = std::cell::RefCell::new(Vec::<String>::new());
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            // Complete fragments, no sentinel: the shape a restart leaves.
            &[
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"apply_patch","arguments":"{\"path\":\"a.rs\"}"}}]}}]}"#,
                "",
            ]
            .join("\n\n"),
        );
        let backend = runtime("coder", &upstream.binding());

        let Err(error) = backend.generate_streaming(&[b"go"], &mut |event: StreamEvent<'_>| {
            if let StreamEvent::ToolCall(call) = event {
                emitted.borrow_mut().push(call.name.clone());
            }
            StreamControl::Continue
        }) else {
            panic!("a stream ending before `[DONE]` is truncated, not complete");
        };
        assert!(error.to_string().contains("[DONE]"), "{error}");
        assert!(
            emitted.borrow().is_empty(),
            "nothing may be dispatched from a stream that then fails: {:?}",
            emitted.borrow()
        );
    }

    /// A present-but-wrong counter is a malformed response, not a zero.
    #[test]
    fn a_malformed_usage_counter_fails_the_response_instead_of_reading_as_zero() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"choices":[{"message":{"content":"hi"}}],
                "usage":{"prompt_tokens":"100","completion_tokens":5}}"#,
        );
        let backend = runtime("coder", &upstream.binding());
        let Err(error) = backend.generate(&[b"go"]) else {
            panic!("a non-integer usage counter must not be reported as measured");
        };
        let error = error.to_string();
        assert!(error.contains("usage.prompt_tokens"), "{error}");
    }

    /// The counters cross the WIT boundary as `u32`; an unchecked cast turned
    /// `4294967297` into `1`, which is worse than refusing it.
    #[test]
    fn a_usage_counter_beyond_u32_fails_rather_than_wrapping() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"choices":[{"message":{"content":"hi"}}],
                "usage":{"prompt_tokens":4294967297,"completion_tokens":5}}"#,
        );
        let backend = runtime("coder", &upstream.binding());
        let Err(error) = backend.generate(&[b"go"]) else {
            panic!("a usage counter beyond u32 must not wrap into a plausible one");
        };
        let error = error.to_string();
        assert!(error.contains("beyond the range"), "{error}");
    }

    /// `guest-openai` renders an absent finish reason as `stop`, so silently
    /// dropping a malformed one reports a truncated answer as complete.
    #[test]
    fn a_non_string_finish_reason_fails_rather_than_reading_as_absent() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"choices":[{"message":{"content":"hi"},"finish_reason":42}]}"#,
        );
        let backend = runtime("coder", &upstream.binding());
        let Err(error) = backend.generate(&[b"go"]) else {
            panic!("a non-string finish reason must not be read as absence");
        };
        let error = error.to_string();
        assert!(error.contains("finish_reason"), "{error}");
    }

    /// Last-write-wins let a `length` be overwritten by a later `stop` — the
    /// one direction that turns a truncated answer into a complete-looking one.
    #[test]
    fn conflicting_streamed_finish_reasons_fail_the_stream() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            &[
                r#"data: {"choices":[{"delta":{"content":"half"},"finish_reason":"length"}]}"#,
                r#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
                "data: [DONE]",
                "",
            ]
            .join("\n\n"),
        );
        let backend = runtime("coder", &upstream.binding());
        let Err(error) =
            backend.generate_streaming(&[b"go"], &mut |_: StreamEvent<'_>| StreamControl::Continue)
        else {
            panic!("two different finish reasons for one choice must fail the stream");
        };
        let error = error.to_string();
        assert!(error.contains("conflicting finish reasons"), "{error}");
    }

    /// Two identities under one index means two calls merged into one slot, and
    /// the arguments accumulated so far belong to whichever this is not.
    #[test]
    fn a_second_identity_for_one_streamed_tool_call_index_fails_the_stream() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            &[
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c2","function":{"arguments":"\"a.rs\"}"}}]}}]}"#,
                "data: [DONE]",
                "",
            ]
            .join("\n\n"),
        );
        let backend = runtime("coder", &upstream.binding());
        let Err(error) =
            backend.generate_streaming(&[b"go"], &mut |_: StreamEvent<'_>| StreamControl::Continue)
        else {
            panic!("a second id for one tool-call index must fail the stream");
        };
        let error = error.to_string();
        assert!(error.contains("second id"), "{error}");
    }

    /// A stream that ends mid-value leaves `{"path":` behind — a string, and
    /// half a call. Passed through it dispatches as a successful invocation
    /// whose arguments no client can parse.
    #[test]
    fn streamed_tool_call_arguments_that_are_not_a_json_object_fail_the_stream() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            &[
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"read_file","arguments":"{\"path\":"}}]}}]}"#,
                "data: [DONE]",
                "",
            ]
            .join("\n\n"),
        );
        let backend = runtime("coder", &upstream.binding());
        let Err(error) =
            backend.generate_streaming(&[b"go"], &mut |_: StreamEvent<'_>| StreamControl::Continue)
        else {
            panic!("incomplete argument JSON must not dispatch as a usable call");
        };
        let error = error.to_string();
        assert!(error.contains("not valid"), "{error}");
    }

    /// A scalar frame parses as JSON and then answers `None` to every lookup,
    /// so its text vanished and `[DONE]` completed the request successfully.
    #[test]
    fn a_non_object_sse_frame_fails_rather_than_reading_as_empty() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            &["data: \"answer\"", "data: [DONE]", ""].join("\n\n"),
        );
        let backend = runtime("coder", &upstream.binding());
        let Err(error) =
            backend.generate_streaming(&[b"go"], &mut |_: StreamEvent<'_>| StreamControl::Continue)
        else {
            panic!("a scalar data frame must not read as an empty event");
        };
        let error = error.to_string();
        assert!(error.contains("must be a JSON object"), "{error}");
    }

    /// Same one level in: `{"choices":[42]}` read as a choice that merely
    /// omitted its delta.
    #[test]
    fn a_non_object_streamed_choice_fails_rather_than_reading_as_empty() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            &["data: {\"choices\":[42]}", "data: [DONE]", ""].join("\n\n"),
        );
        let backend = runtime("coder", &upstream.binding());
        let Err(error) =
            backend.generate_streaming(&[b"go"], &mut |_: StreamEvent<'_>| StreamControl::Continue)
        else {
            panic!("a non-object choice must not read as an empty one");
        };
        let error = error.to_string();
        assert!(error.contains("must be an object"), "{error}");
    }

    #[test]
    fn a_buffered_tool_call_arrives_structured_rather_than_encoded_into_the_text() {
        // Smuggled through the text channel, a call decodes only when the
        // client happens to pass the nonstandard parser option, so a standard
        // OpenAI client would receive the JSON as literal assistant prose.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"choices":[{"message":{"content":null,"tool_calls":[
                {"id":"c1","type":"function","function":{"name":"f","arguments":"{}"}}]}}]}"#,
        );
        let backend = runtime("coder", &upstream.binding());
        let generation = backend.generate(&[b"go"]).expect("tool call round trip");
        assert!(
            generation.bytes.is_empty(),
            "a call carries no assistant text, and none must be invented"
        );
        assert_eq!(
            generation.tool_calls,
            vec![ToolCall {
                id: Some("c1".to_owned()),
                name: "f".to_owned(),
                arguments: "{}".to_owned(),
            }]
        );
    }

    #[test]
    fn an_oversized_sse_frame_is_refused_during_the_read() {
        // One frame with no newline at all: the read itself must stop at the
        // frame limit rather than growing to the whole-stream limit.
        let mut body = String::from("data: {\"choices\":[{\"delta\":{\"content\":\"");
        body.push_str(&"x".repeat(MAX_SSE_FRAME_BYTES + 16));
        let upstream = FakeUpstream::start("HTTP/1.1 200 OK", "text/event-stream", &body);
        let backend = runtime("coder", &upstream.binding());

        let error = backend
            .generate_streaming(&[b"hi"], &mut |_: StreamEvent<'_>| StreamControl::Continue)
            .expect_err("an oversized frame must be refused");
        assert!(error.to_string().contains("limit"), "got: {error}");
    }

    #[test]
    fn a_malformed_sse_frame_fails_the_request() {
        // Skipping the frame would truncate the answer while `[DONE]` reported
        // the request as successful.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"fn \"}}]}\n\n",
                "data: {not json}\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());
        let error = backend
            .generate_streaming(&[b"hi"], &mut |_: StreamEvent<'_>| StreamControl::Continue)
            .expect_err("a malformed data frame must not be skipped");
        assert!(matches!(error, UpstreamError::MalformedResponse { .. }));
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

        let mut captured = CapturedStream::default();
        let error = backend
            .generate_streaming(&[b"write main"], &mut captured.sink())
            .expect_err("a stream without [DONE] is truncated, not complete");
        assert!(matches!(error, UpstreamError::MalformedResponse { .. }));
        assert!(
            error.to_string().contains("[DONE]"),
            "the error should name the missing sentinel, got: {error}"
        );
        // Tokens already forwarded are not retracted; only the outcome changes.
        assert_eq!(captured.content, vec!["fn ", "main"]);
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
        let (body, _timeout) = backend
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
        let (body, _timeout) = backend
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

        let generation = backend
            .generate(&[b"read a.rs"])
            .expect("a tool call is a successful generation");
        assert_eq!(
            generation.tool_calls,
            vec![ToolCall {
                id: Some("call_1".to_owned()),
                name: "read_file".to_owned(),
                arguments: "{\"path\":\"a.rs\"}".to_owned(),
            }]
        );
        assert!(generation.bytes.is_empty());
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

        let mut captured = CapturedStream::default();
        backend
            .generate_streaming(&[b"read a.rs"], &mut captured.sink())
            .expect("streaming should complete");
        assert!(
            captured.content.is_empty(),
            "a tool-call-only stream carries no assistant text"
        );
        assert_eq!(
            captured.tool_calls,
            vec![ToolCall {
                id: Some("call_1".to_owned()),
                name: "read_file".to_owned(),
                arguments: "{\"path\":\"a.rs\"}".to_owned(),
            }]
        );
    }

    #[test]
    fn tool_call_arguments_survive_an_upstream_that_sends_an_object() {
        // OpenAI specifies `function.arguments` as a JSON *string*, but several
        // compatible servers send the object. Reading only the string form
        // dispatched the right function with `{}` — the worst shape of failure,
        // because the request succeeds and the tool runs on nothing.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"choices":[{"message":{"content":null,"tool_calls":[
                {"id":"c1","type":"function",
                 "function":{"name":"read_file","arguments":{"path":"a.rs"}}}]}}]}"#,
        );
        let backend = runtime("coder", &upstream.binding());
        let generation = backend.generate(&[b"go"]).expect("tool call round trip");
        let arguments: Value = serde_json::from_str(&generation.tool_calls[0].arguments)
            .expect("the arguments must still be JSON");
        assert_eq!(arguments["path"], "a.rs");
    }

    #[test]
    fn a_streamed_object_argument_is_kept_rather_than_dropped() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"read_file","arguments":{"path":"a.rs"}}}]}}]}"#,
                "\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut captured = CapturedStream::default();
        backend
            .generate_streaming(&[b"go"], &mut captured.sink())
            .expect("streaming should complete");
        let arguments: Value = serde_json::from_str(&captured.tool_calls[0].arguments)
            .expect("the arguments must still be JSON");
        assert_eq!(arguments["path"], "a.rs");
    }

    #[test]
    fn an_argument_value_that_is_neither_string_nor_object_fails_the_response() {
        // Reducing a number to `{}` would be the same silent substitution the
        // two tests above exist to prevent.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"choices":[{"message":{"content":null,"tool_calls":[
                {"id":"c1","type":"function","function":{"name":"f","arguments":7}}]}}]}"#,
        );
        let backend = runtime("coder", &upstream.binding());
        let error = backend
            .generate(&[b"go"])
            .expect_err("an unusable argument value is not an empty one");
        assert!(matches!(error, UpstreamError::MalformedResponse { .. }));
    }

    #[test]
    fn a_malformed_streamed_tool_call_payload_fails_the_stream() {
        // An object where an array belongs used to read as "no tool calls",
        // so the stream completed successfully — carrying the upstream's own
        // `finish_reason: "tool_calls"` — with nothing for the client to
        // dispatch. The buffered path already rejected this shape.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                r#"data: {"choices":[{"delta":{"tool_calls":{"name":"read_file"}},"finish_reason":"tool_calls"}]}"#,
                "\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut captured = CapturedStream::default();
        let error = backend
            .generate_streaming(&[b"go"], &mut captured.sink())
            .expect_err("a tool-call payload that cannot be read is not an empty one");
        assert!(matches!(error, UpstreamError::MalformedResponse { .. }));
        assert!(
            error.to_string().contains("must be an array"),
            "the error should name the shape problem, got: {error}"
        );
    }

    #[test]
    fn a_streamed_choices_value_that_is_not_an_array_fails_the_stream() {
        // `as_array` alone turned a present object into *no choice at all*, so
        // the frame's content was dropped, `[DONE]` still completed the stream,
        // and the guest reported an ordinary `stop` over a truncated answer.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                r#"data: {"choices":{"delta":{"content":"answer"}}}"#,
                "\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut captured = CapturedStream::default();
        let error = backend
            .generate_streaming(&[b"go"], &mut captured.sink())
            .expect_err("a non-array `choices` is not an absent one");
        assert!(matches!(error, UpstreamError::MalformedResponse { .. }));
        assert!(
            error.to_string().contains("non-array `choices`"),
            "the error should name the shape problem, got: {error}"
        );
        assert!(
            captured.content.is_empty(),
            "nothing may be emitted from a frame that could not be read"
        );
    }

    #[test]
    fn a_streamed_delta_that_is_not_an_object_fails_the_stream() {
        // One level below the `choices` case: a present non-object `delta` was
        // accepted, and every `get` below it then read neither content nor tool
        // call, so the frame was discarded whole and `[DONE]` still completed
        // the stream on an ordinary `stop`.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                r#"data: {"choices":[{"delta":"answer"}]}"#,
                "\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut captured = CapturedStream::default();
        let error = backend
            .generate_streaming(&[b"go"], &mut captured.sink())
            .expect_err("a non-object delta is not an absent one");
        assert!(matches!(error, UpstreamError::MalformedResponse { .. }));
        assert!(
            error.to_string().contains("non-object `delta`"),
            "the error should name the shape problem, got: {error}"
        );
        assert!(captured.content.is_empty());
    }

    #[test]
    fn a_final_frame_without_a_delta_is_not_an_error() {
        // The other side: a frame that only reports `finish_reason` carries no
        // `delta` at all, and rejecting it would fail ordinary streams.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                r#"data: {"choices":[{"delta":{"content":"hi"}}]}"#,
                "\n\n",
                r#"data: {"choices":[{"finish_reason":"stop"}]}"#,
                "\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut captured = CapturedStream::default();
        let outcome = backend
            .generate_streaming(&[b"go"], &mut captured.sink())
            .expect("a final frame carries no delta and is not malformed");
        assert_eq!(captured.content.concat(), "hi");
        assert_eq!(outcome.finish_reason.as_deref(), Some("stop"));
    }

    #[test]
    fn a_usage_only_frame_without_choices_is_not_an_error() {
        // The other side of the rule: absent `choices` is the normal shape of a
        // usage frame, and rejecting it would fail every stream that reports
        // token counts.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                r#"data: {"choices":[{"delta":{"content":"hi"}}]}"#,
                "\n\n",
                r#"data: {"usage":{"prompt_tokens":3,"completion_tokens":1}}"#,
                "\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut captured = CapturedStream::default();
        let outcome = backend
            .generate_streaming(&[b"go"], &mut captured.sink())
            .expect("a usage frame carries no choices and is not malformed");
        assert_eq!(captured.content.concat(), "hi");
        assert_eq!(
            outcome.usage.expect("usage is reported").completion_tokens,
            1
        );
    }

    #[test]
    fn a_buffered_content_value_that_is_not_a_string_fails_beside_tool_calls() {
        // With valid `tool_calls` present, `as_str` reported a *present* object
        // as absent and the branch returned empty bytes while reporting a
        // successful structured call — the assistant's message silently gone.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            concat!(
                r#"{"choices":[{"message":{"content":{"text":"hi"},"tool_calls":"#,
                r#"[{"id":"call_1","type":"function","function":{"name":"read_file","arguments":"{}"}}]"#,
                r#"}}]}"#,
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let error = backend
            .generate(&[b"go"])
            .expect_err("a non-string content value is not an absent one");
        assert!(matches!(error, UpstreamError::MalformedResponse { .. }));
        assert!(
            error.to_string().contains("non-string"),
            "the error should name the shape problem, got: {error}"
        );
    }

    #[test]
    fn a_streamed_content_value_that_is_not_a_string_fails_the_stream() {
        // Reading this with `as_str` alone reported a *present* object as
        // absent, so the delta was dropped and the stream still finished
        // successfully. The client got a silently short answer with nothing
        // marking it short — the buffered path rejects the same shape.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                r#"data: {"choices":[{"delta":{"content":{"text":"hi"}}}]}"#,
                "\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut captured = CapturedStream::default();
        let error = backend
            .generate_streaming(&[b"go"], &mut captured.sink())
            .expect_err("a non-string content value is not an absent one");
        assert!(matches!(error, UpstreamError::MalformedResponse { .. }));
        assert!(
            error.to_string().contains("expected a string"),
            "the error should name the shape problem, got: {error}"
        );
    }

    #[test]
    fn a_streamed_null_content_is_still_an_ordinary_frame() {
        // `content: null` is how a tool-call or role-only frame carries no
        // text. Rejecting it would fail every tool call, so absence and
        // malformation must stay distinguishable.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                r#"data: {"choices":[{"delta":{"role":"assistant","content":null}}]}"#,
                "\n\n",
                r#"data: {"choices":[{"delta":{"content":"ok"}}]}"#,
                "\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut captured = CapturedStream::default();
        backend
            .generate_streaming(&[b"go"], &mut captured.sink())
            .expect("a null content frame is not a failure");
        assert_eq!(captured.content.concat(), "ok");
    }

    #[test]
    fn streamed_tool_call_fragments_without_an_index_fail_the_stream() {
        // Defaulting a missing index to 0 merged two distinct calls into one
        // slot: the second id and name overwrote the first while the arguments
        // concatenated across both, so the caller would dispatch one function
        // with another's arguments.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                r#"data: {"choices":[{"delta":{"tool_calls":[{"id":"a","function":{"name":"read","arguments":"{}"}}]}}]}"#,
                "\n\n",
                r#"data: {"choices":[{"delta":{"tool_calls":[{"id":"b","function":{"name":"write","arguments":"{}"}}]}}]}"#,
                "\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut captured = CapturedStream::default();
        let error = backend
            .generate_streaming(&[b"go"], &mut captured.sink())
            .expect_err("a fragment with no index cannot be attributed to a call");
        assert!(matches!(error, UpstreamError::MalformedResponse { .. }));
        assert!(
            error.to_string().contains("no `index`"),
            "the error should name the missing field, got: {error}"
        );
    }

    #[test]
    fn a_streamed_tool_call_index_of_the_wrong_type_fails_the_stream() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":"0","id":"a","function":{"name":"read"}}]}}]}"#,
                "\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut captured = CapturedStream::default();
        let error = backend
            .generate_streaming(&[b"go"], &mut captured.sink())
            .expect_err("a string index is not an integer one");
        assert!(matches!(error, UpstreamError::MalformedResponse { .. }));
    }

    #[test]
    fn an_upstream_embedding_is_normalized_to_unit_length() {
        // `embed` promises an L2-normalized vector and the local backend
        // delivers one, so a consumer using a dot product as cosine similarity
        // is reading the contract correctly. Returning `[3, 4]` unchanged made
        // rankings magnitude-dependent for upstream-bound aliases only.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"data":[{"embedding":[3.0,4.0]}]}"#,
        );
        let backend = runtime("embed", &upstream.binding());

        let vector = backend.embed("hello").expect("embedding should round trip");
        assert!((vector[0] - 0.6).abs() < 1e-6, "got {}", vector[0]);
        assert!((vector[1] - 0.8).abs() < 1e-6, "got {}", vector[1]);
        let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "norm should be 1, got {norm}");
    }

    #[test]
    fn a_zero_norm_upstream_embedding_is_rejected() {
        // It cannot be normalized at all, and every cosine similarity against
        // it is undefined — so returning it would satisfy the call and break
        // the contract the caller is relying on.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"data":[{"embedding":[0.0,0.0,0.0]}]}"#,
        );
        let backend = runtime("embed", &upstream.binding());

        let error = backend
            .embed("hello")
            .expect_err("a zero vector cannot satisfy the unit-length contract");
        assert!(matches!(error, UpstreamError::MalformedResponse { .. }));
    }

    /// A sink that emits nothing and reports the consumer gone from the start,
    /// standing in for a client that disconnected before the first content
    /// delta arrived.
    #[derive(Default)]
    struct DepartedSink {
        emitted: usize,
    }

    impl StreamSink for DepartedSink {
        fn emit(&mut self, _event: StreamEvent<'_>) -> StreamControl {
            self.emitted += 1;
            StreamControl::Stop
        }

        fn is_live(&mut self) -> bool {
            false
        }
    }

    #[test]
    fn a_stream_carrying_no_content_still_notices_a_departed_client() {
        // Role-only openings, usage frames and keep-alives produce no event, so
        // `emit` is never called and cannot report the departure. Without a
        // liveness probe per frame this ran to completion — holding a socket, a
        // thread and an admission permit — for a client that had already gone.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#,
                "\n\n",
                r#"data: {"choices":[],"usage":{"prompt_tokens":3,"completion_tokens":0}}"#,
                "\n\n",
                r#"data: {"choices":[{"delta":{"content":"never read"}}]}"#,
                "\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut sink = DepartedSink::default();
        backend
            .generate_streaming(&[b"hi"], &mut sink)
            .expect("an abandoned stream is not an error");
        assert_eq!(
            sink.emitted, 0,
            "the read must stop on the first frame, before any content is produced"
        );
    }

    #[test]
    fn a_departed_client_stops_the_upstream_read() {
        // Three content frames and no `[DONE]`: a stream this backend would
        // normally reject as truncated. With the consumer gone that is the
        // intended outcome, and the point is that the read stops at the first
        // frame instead of draining the rest — which is what releases the
        // socket, the thread and the admission permit.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"b\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"c\"}}]}\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut captured = CapturedStream::default();
        backend
            .generate_streaming(&[b"hi"], &mut captured.sink_stopping_after(1))
            .expect("an abandoned stream is not an error");
        assert_eq!(
            captured.content,
            vec!["a"],
            "the read must stop at the frame the sink refused, not drain the rest"
        );
    }

    #[test]
    fn a_streamed_finish_reason_survives_to_the_caller() {
        // `length` means the upstream truncated the answer at its own token
        // limit. Discarding it on the streaming path reported the completion as
        // `stop`, so a client ran half a function.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"fn main() {\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut captured = CapturedStream::default();
        let outcome = backend
            .generate_streaming(&[b"write main"], &mut captured.sink())
            .expect("streaming should complete");
        assert_eq!(outcome.finish_reason.as_deref(), Some("length"));
    }

    #[test]
    fn a_stream_without_tool_calls_reports_none() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());

        let mut captured = CapturedStream::default();
        backend
            .generate_streaming(&[b"hi"], &mut captured.sink())
            .expect("streaming should complete");
        assert_eq!(captured.content, vec!["ok"]);
        assert!(captured.tool_calls.is_empty());
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
        let (body, _timeout) = backend
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
            .generate_streaming(&[b"hi"], &mut |_: StreamEvent<'_>| StreamControl::Continue)
            .expect_err("an in-band error must not be reported as success");
        assert!(error.to_string().contains("context length exceeded"));
    }

    #[test]
    fn offering_tools_no_longer_withholds_streamed_content() {
        // Content then tool calls in the same stream. With calls confined to
        // the text channel this had to be answered with a single envelope
        // emitted at `[DONE]`, so every tool-enabled request paid the whole
        // generation in time-to-first-token — including the ones that never
        // called anything. The prose must now arrive as it is produced, and the
        // call beside it.
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

        let mut captured = CapturedStream::default();
        backend
            .generate_streaming(
                &[br#"{"prompt":"read","tools":[{"type":"function","function":{"name":"read_file"}}]}"#],
                &mut captured.sink(),
            )
            .expect("streaming should complete");
        assert_eq!(
            captured.content,
            vec!["Let me look. "],
            "prose must reach the caller as a content event, not buffered into a trailing envelope"
        );
        assert_eq!(
            captured.tool_calls,
            vec![ToolCall {
                id: Some("c1".to_owned()),
                name: "read_file".to_owned(),
                arguments: "{}".to_owned(),
            }]
        );
    }

    #[test]
    fn colliding_credential_variables_are_refused() {
        // `-`, `_` and `.` all collapse to `_`, so these three name one
        // variable. Allowing them would send one third party's bearer token to
        // another's server.
        for pair in [
            ["vendor-a", "vendor_a"],
            ["vendor.a", "vendor-a"],
            ["Vendor-A", "vendor_a"],
        ] {
            let error = assert_no_credential_collisions(pair)
                .expect_err("colliding aliases must fail validation");
            assert!(
                error.contains(API_KEY_ENV_PREFIX),
                "the error should name the shared variable, got: {error}"
            );
        }

        // Distinct suffixes are fine, including aliases that differ only after
        // normalization.
        assert!(assert_no_credential_collisions(["vendor-a", "vendor-b", "other"]).is_ok());
        assert!(assert_no_credential_collisions(std::iter::empty()).is_ok());
    }

    #[test]
    fn an_empty_per_alias_key_falls_back_to_the_global_one() {
        // What optional secret injection produces when the secret is missing:
        // the variable exists and is empty. Treating that as a hit skipped the
        // global fallback and sent the request unauthenticated.
        let alias = "empty-key-probe";
        let per_alias = format!("{API_KEY_ENV_PREFIX}{}", api_key_env_suffix(alias));
        // SAFETY: single-threaded test process section; both variables are
        // removed before returning.
        unsafe {
            env::set_var(&per_alias, "   ");
            env::set_var(API_KEY_ENV_FALLBACK, "global-secret");
        }
        let resolved = api_key_for(alias);
        unsafe {
            env::remove_var(&per_alias);
            env::remove_var(API_KEY_ENV_FALLBACK);
        }
        assert_eq!(resolved.as_deref(), Some("global-secret"));
    }

    #[test]
    fn a_truncated_upstream_completion_reports_length() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "application/json",
            r#"{"choices":[{"finish_reason":"length","message":{"content":"fn main() {"}}]}"#,
        );
        let backend = runtime("coder", &upstream.binding());
        let generation = backend.generate(&[b"write main"]).expect("generation");
        assert_eq!(
            generation.finish_reason.as_deref(),
            Some("length"),
            "a truncated completion must not look complete"
        );
    }

    #[test]
    fn a_malformed_tool_call_array_is_rejected() {
        // Forwarding it would hand the caller the internal marker envelope as
        // ordinary assistant content, with HTTP 200.
        for payload in [
            r#"{"choices":[{"message":{"content":null,"tool_calls":{"not":"an array"}}}]}"#,
            r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"c1","type":"function"}]}}]}"#,
        ] {
            let upstream = FakeUpstream::start("HTTP/1.1 200 OK", "application/json", payload);
            let backend = runtime("coder", &upstream.binding());
            let error = backend
                .generate(&[b"go"])
                .expect_err("an unusable tool-call payload must fail");
            assert!(
                matches!(error, UpstreamError::MalformedResponse { .. }),
                "expected MalformedResponse, got {error:?}"
            );
        }
    }

    #[test]
    fn an_incomplete_streamed_tool_call_fails_the_response() {
        // Fragments arrived, so the upstream committed to a call; one that
        // never names its function is a truncated stream, not a call to drop.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"arguments":"{}"}}]}}]}"#,
                "\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());
        let error = backend
            .generate_streaming(
                &[br#"{"prompt":"go","tools":[{"type":"function"}]}"#],
                &mut |_token: StreamEvent<'_>| StreamControl::Continue,
            )
            .expect_err("an unnamed streamed call must fail the response");
        assert!(
            error.to_string().contains("never sent a function name"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn a_fragment_after_the_query_is_rejected() {
        // Written in its normal position, the fragment used to land inside the
        // query half and silently become part of the last parameter's value.
        let error = UpstreamEndpoint::parse("coder", "openai:http://host/v1?model=foo#bar")
            .expect_err("a fragment must be refused wherever it appears");
        assert!(
            error.to_string().contains("fragment"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn an_upstream_that_reports_no_usage_reports_none_rather_than_zero() {
        // The distinction the whole `Option` exists for. Most upstreams only
        // emit a usage frame when asked, and this backend deliberately does not
        // ask — so absence is the common case, and collapsing it to zeros would
        // tell every client that these generations cost nothing.
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());
        let outcome = backend
            .generate_streaming(&[b"hi"], &mut |_token: StreamEvent<'_>| {
                StreamControl::Continue
            })
            .expect("streaming should complete");
        assert_eq!(outcome.usage, None);
    }

    #[test]
    fn an_upstream_usage_frame_is_reported_verbatim() {
        let upstream = FakeUpstream::start(
            "HTTP/1.1 200 OK",
            "text/event-stream",
            concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n",
                // The usage frame carries no choices, so it must be read before
                // the delta lookup skips it.
                "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":4}}\n\n",
                "data: [DONE]\n\n",
            ),
        );
        let backend = runtime("coder", &upstream.binding());
        let outcome = backend
            .generate_streaming(&[b"hi"], &mut |_token: StreamEvent<'_>| {
                StreamControl::Continue
            })
            .expect("streaming should complete");
        assert_eq!(
            outcome.usage,
            Some(TokenUsage {
                prompt_tokens: 11,
                completion_tokens: 4,
            })
        );
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
        let mut captured = CapturedStream::default();
        backend
            .generate_streaming(&[b"hi"], &mut captured.sink())
            .expect("streaming should complete");
        assert_eq!(captured.content, vec!["a", "b"]);
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
