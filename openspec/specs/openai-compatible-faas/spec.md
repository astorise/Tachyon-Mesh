# openai-compatible-faas Specification

## Purpose
Define the user-space OpenAI-compatible FaaS surface, its model registry
ownership, streaming behavior, and the decoupled static chat UI example that
dogfoods Tachyon FaaS for browser delivery.
## Requirements
### Requirement: User-space OpenAI-compatible FaaS

The OpenAI-compatible HTTP surface SHALL be provided by a single **user-role**
FaaS (`guest-openai`) built against the `faas-guest` WIT world. It SHALL expose
`GET /ai/v1/models`, `POST /ai/v1/chat/completions`, and
`POST /ai/v1/embeddings`, and it SHALL NOT expose the former public `/v1/models`,
`/v1/chat/completions`, or `/v1/embeddings` routes. It SHALL NOT be a system FaaS
injected by a compile-time feature flag. A dynamic model advertised by the
registry SHALL be usable by the chat and embeddings routes when that alias is
included in the route's sealed dynamic model bindings.

#### Scenario: Model listing returns OpenAI-compatible shape

- **GIVEN** the registry contains at least one available model
- **WHEN** a client requests `GET /ai/v1/models`
- **THEN** `guest-openai` returns a JSON body with `object: "list"` and a `data` array
- **AND** each item includes an `id`, `object: "model"`, and `owned_by: "tachyon-mesh"`

#### Scenario: Chat completions runs real inference

- **GIVEN** a sealed static or dynamic model alias the route is allowed to use
- **WHEN** a client requests `POST /ai/v1/chat/completions` naming that model
- **THEN** `guest-openai` loads the model on the requested accelerator, hands
  the structured conversation and sampling parameters to the host, and returns
  an OpenAI-shaped `chat.completion`

#### Scenario: Embeddings returns OpenAI-compatible vectors

- **GIVEN** a sealed static or dynamic model alias the route is allowed to use
- **AND** that alias resolves to an ONNX embedding model directory containing
  `tokenizer.json` and a model file such as `model.onnx`
- **WHEN** a client requests `POST /ai/v1/embeddings` naming that model with a
  single string or list of strings
- **THEN** `guest-openai` loads the model on the CPU accelerator, calls the host
  Candle ONNX embeddings primitive once per input, applies masked pooling and
  L2 normalization, and returns an OpenAI-shaped `list` of `embedding` objects
  preserving input order

#### Scenario: Listed dynamic model is authorized for chat

- **GIVEN** a dynamic model is registered and listed by `/ai/v1/models`
- **AND** the `/ai/v1/chat/completions` route contains a sealed dynamic binding for
  its alias
- **WHEN** a client requests a chat completion with that alias
- **THEN** the request SHALL pass route model authorization

#### Scenario: Former public routes are absent

- **WHEN** a client requests `GET /v1/models`, `POST /v1/chat/completions`, or
  `POST /v1/embeddings`
- **THEN** the node returns `404` because those routes are not sealed

#### Scenario: Unknown model returns 404

- **WHEN** a chat completion request names a model the host cannot load
- **THEN** `guest-openai` responds with HTTP `404` and an OpenAI-shaped
  `model_not_found` error body

#### Scenario: Present without feature injection

- **GIVEN** a node whose sealed manifest declares the `guest-openai` user routes
- **WHEN** the topology graph is built from that manifest
- **THEN** `guest-openai` appears as a user route regardless of whether the
  binary was compiled with `ai-inference`

### Requirement: Registry ownership via kv-partition

`guest-openai` SHALL own the model registry by reading and writing the `ai-models-registry` `kv-partition` table directly. It SHALL serve the register, list, and deregister operations previously served by `system-faas-ai-list-model`, with no separate registry FaaS and no outbound mesh call to list models.

#### Scenario: Register persists a model

- **WHEN** a register request is received with a valid model record
- **THEN** `guest-openai` writes the record to the `ai-models-registry` table keyed by alias
- **AND** the model becomes listable

#### Scenario: Deregister removes a model

- **WHEN** a deregister request is received for an existing alias
- **THEN** `guest-openai` deletes that key from the `ai-models-registry` table
- **AND** the model no longer appears in the listing

### Requirement: Fresh registry reads

`guest-openai` SHALL read the `ai-models-registry` table on every list request and SHALL NOT cache the model list in guest memory across requests, so that a model registered on any instance is visible on the next list from any instance.

#### Scenario: Newly registered model is visible immediately

- **GIVEN** a model is registered
- **WHEN** `GET /ai/v1/models` is served by the same instance immediately afterward
- **THEN** the response includes the newly registered model

#### Scenario: Visibility across instances

- **GIVEN** a model is registered on instance A
- **WHEN** `GET /ai/v1/models` is served by a different instance B
- **THEN** the response includes the model registered on A

### Requirement: Scope-gated registry table access

The `guest-openai` route SHALL declare deployment scopes that grant `kv` access to the `ai-models-registry` table. Table access is gated by deployment scopes, not by guest role; a guest without that grant SHALL be denied access to the table.

#### Scenario: Granted route opens the table

- **GIVEN** the `guest-openai` route declares a `scopes.kv` grant for `ai-models-registry`
- **WHEN** it opens the table
- **THEN** the open succeeds and reads/writes proceed

#### Scenario: Ungranted guest is denied

- **GIVEN** a guest route without a `scopes.kv` grant for `ai-models-registry`
- **WHEN** it attempts to open the table
- **THEN** the host denies the open with a scope-denial error and records a scope denial

### Requirement: Upload notification persists to the shared registry

`system-faas-model-broker` SHALL notify `guest-openai` over HTTP when a newly uploaded model becomes available, targeting the internal register endpoint, so the model is recorded in the shared `ai-models-registry` table and becomes listable.

#### Scenario: Broker upload makes a model listable

- **WHEN** `model-broker` completes a model upload and notifies `guest-openai`'s register endpoint
- **THEN** the model is written to `ai-models-registry`
- **AND** a subsequent `GET /ai/v1/models` includes that model

### Requirement: Chat completion sampling parameters

`guest-openai` SHALL forward the OpenAI sampling parameters of a chat completion
request to the host: `temperature`, `top_p`, `seed`, `max_tokens`, and `stop`.
The conversation SHALL be forwarded as structured `messages` (the host renders
the model's chat template), not pre-flattened. Parameters that are unset SHALL be
omitted so the host applies its own defaults. A `stop` value given as a single
string SHALL be normalised to a list.

#### Scenario: Sampling parameters reach the host

- **WHEN** a chat completion request sets `temperature`, `top_p`, `seed`, or
  `stop`
- **THEN** `guest-openai` includes those values in the host generation request
- **AND** forwards the conversation as structured `messages`

#### Scenario: Scalar stop is normalised

- **WHEN** a request supplies `stop` as a single string
- **THEN** `guest-openai` forwards it as a one-element list

#### Scenario: Unset parameters are omitted

- **WHEN** a request omits a sampling parameter
- **THEN** `guest-openai` omits it from the host generation request so the host
  default applies

### Requirement: Chat completion tool calling post-processing

`guest-openai` SHALL support OpenAI-compatible tool calling for buffered chat
completion responses as user-space post-processing of generated assistant text.
When a request provides `tools` or `tool_choice`, it SHALL select a configurable
tool-call parser from `tool_call_parser`, `extra_body.tool_call_parser`, or a
model-name heuristic for `qwen_coder`, `qwen`, and `mistral`. Supported parser
names SHALL include `json`, `qwen_coder`, `qwen`, and `mistral`. Parsed calls
SHALL be returned as `message.tool_calls` entries with `type: "function"`,
canonical string `function.arguments`, and `finish_reason: "tool_calls"`.
Unparseable model output SHALL fall back to normal assistant `content` with no
tool calls. This parsing SHALL remain outside the host decode loop.

#### Scenario: Tools and parser reach the generation request

- **WHEN** a buffered chat completion request includes `tools` and a tool-call
  parser is explicitly or implicitly selected
- **THEN** `guest-openai` forwards `tools`, `tool_choice`, and
  `tool_call_parser` in the structured host generation request

#### Scenario: JSON parser emits OpenAI tool calls

- **WHEN** generated assistant text is JSON containing `tool_calls`
- **THEN** `guest-openai` converts each call into an OpenAI-compatible
  `message.tool_calls` item
- **AND** sets the choice `finish_reason` to `tool_calls`

#### Scenario: Qwen and Mistral parser formats are supported

- **WHEN** generated assistant text contains Qwen `<tool_call>` tags or a
  Mistral `[TOOL_CALLS]` payload
- **THEN** `guest-openai` extracts the function name and arguments into
  OpenAI-compatible tool calls

#### Scenario: Tool parsing is inactive without tool intent

- **WHEN** a chat completion request omits both `tools` and `tool_choice`
- **THEN** generated text that resembles a tool-call payload is returned as
  normal assistant `content`

### Requirement: Streaming chat completions

When a chat completion request sets `stream: true`, `guest-openai` SHALL respond
with an OpenAI-compatible Server-Sent Events stream: a sequence of
`chat.completion.chunk` frames carrying incremental `delta` content, terminated
by a `data: [DONE]` frame, with `content-type: text/event-stream`. Fragments
SHALL be flushed as the host produces them (pulled from the accelerator
streaming primitive), giving real time-to-first-token. The concatenation of the
streamed deltas SHALL equal the message content of the equivalent non-streamed
response. Because the OpenAI framing is user-space, it is produced by
`guest-openai`; the host provides only a generic incremental body-flush
transport.

#### Scenario: Streamed response is OpenAI-compatible

- **WHEN** a chat completion request sets `stream: true`
- **THEN** `guest-openai` responds with `content-type: text/event-stream`
- **AND** emits `chat.completion.chunk` frames with incremental `delta` content
- **AND** terminates the stream with a `data: [DONE]` frame

#### Scenario: Streamed deltas match the buffered content

- **GIVEN** the same request run with and without `stream: true`
- **WHEN** the streamed `delta` fragments are concatenated
- **THEN** they equal the assistant message content of the non-streamed response

#### Scenario: Non-streaming requests are unchanged

- **WHEN** a chat completion request omits `stream` or sets it to `false`
- **THEN** `guest-openai` returns a single buffered `chat.completion` JSON body

### Requirement: Static chat UI FaaS example

The repository SHALL provide a `guest-chat-ui` user-role FaaS example that
serves a browser chat UI as static assets under `/chat`. The example SHALL be
implemented as a framework-free Web Component (`<tachyon-chat-assistant>`) with
Shadow DOM encapsulation. The component SHALL call the browser-visible
OpenAI-compatible gateway directly, using `GET /ai/v1/models` for model
discovery when available and streamed `POST /ai/v1/chat/completions` requests
for assistant responses.

#### Scenario: Static FaaS serves chat assets

- **WHEN** a browser requests `/chat`
- **THEN** `guest-chat-ui` returns an HTML shell that loads
  `/chat/tachyon-chat-assistant.js`
- **AND** the JavaScript asset defines `<tachyon-chat-assistant>`
- **AND** successful static asset responses include cache headers and ETags

#### Scenario: Web Component streams directly from the gateway

- **WHEN** the user sends a message through `<tachyon-chat-assistant>`
- **THEN** the component sends a streamed OpenAI-compatible request directly to
  `/ai/v1/chat/completions`
- **AND** it consumes `data:` Server-Sent Event frames until `[DONE]`

#### Scenario: Standalone topology includes the complete chat stack

- **WHEN** an operator deploys the `guest-chat-ui` example manifest
- **THEN** the topology includes the static `/chat` route, the
  `/ai/v1/models` route, the `/ai/v1/chat/completions` route, and the internal
  `/ai/v1/embeddings` route, and the internal `guest-openai` registration route
  backed by the shared `ai-models-registry` table

