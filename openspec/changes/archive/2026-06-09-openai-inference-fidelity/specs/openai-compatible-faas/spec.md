## MODIFIED Requirements

### Requirement: User-space OpenAI-compatible FaaS

The OpenAI-compatible HTTP surface SHALL be provided by a single **user-role** FaaS (`guest-openai`) built against the `faas-guest` WIT world. It SHALL expose `GET /v1/models` and `POST /v1/chat/completions`, and it SHALL NOT be a system FaaS injected by a compile-time feature flag.

#### Scenario: Model listing returns OpenAI-compatible shape

- **GIVEN** the registry contains at least one available model
- **WHEN** a client requests `GET /v1/models`
- **THEN** `guest-openai` returns a JSON body with `object: "list"` and a `data` array
- **AND** each item includes an `id`, `object: "model"`, and `owned_by: "tachyon-mesh"`

#### Scenario: Chat completions runs real inference

- **GIVEN** a sealed model alias the route is allowed to use
- **WHEN** a client requests `POST /v1/chat/completions` naming that model
- **THEN** `guest-openai` loads the model on the CPU accelerator, hands the
  structured conversation and sampling parameters to the host, and returns an
  OpenAI-shaped `chat.completion` whose assistant message carries the generated
  text

#### Scenario: Unknown model returns 404

- **WHEN** a chat completion request names a model the host cannot load
- **THEN** `guest-openai` responds with HTTP `404` and an OpenAI-shaped
  `model_not_found` error body

#### Scenario: Present without feature injection

- **GIVEN** a node whose sealed manifest declares the `guest-openai` user routes
- **WHEN** the topology graph is built from that manifest
- **THEN** `guest-openai` appears as a user route regardless of whether the binary was compiled with `ai-inference`

## ADDED Requirements

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
