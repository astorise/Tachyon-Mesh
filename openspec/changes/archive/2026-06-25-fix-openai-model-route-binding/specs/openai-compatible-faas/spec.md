## MODIFIED Requirements

### Requirement: User-space OpenAI-compatible FaaS

The OpenAI-compatible HTTP surface SHALL be provided by a single **user-role**
FaaS (`guest-openai`) built against the `faas-guest` WIT world. It SHALL expose
`GET /v1/models` and `POST /v1/chat/completions`, and it SHALL NOT be a system
FaaS injected by a compile-time feature flag. A dynamic model advertised by the
registry SHALL be usable by the chat route when that alias is included in the
route's sealed dynamic model bindings.

#### Scenario: Model listing returns OpenAI-compatible shape

- **GIVEN** the registry contains at least one available model
- **WHEN** a client requests `GET /v1/models`
- **THEN** `guest-openai` returns a JSON body with `object: "list"` and a `data` array
- **AND** each item includes an `id`, `object: "model"`, and `owned_by: "tachyon-mesh"`

#### Scenario: Chat completions runs real inference

- **GIVEN** a sealed static or dynamic model alias the route is allowed to use
- **WHEN** a client requests `POST /v1/chat/completions` naming that model
- **THEN** `guest-openai` loads the model on the requested accelerator, hands
  the structured conversation and sampling parameters to the host, and returns
  an OpenAI-shaped `chat.completion`

#### Scenario: Listed dynamic model is authorized for chat

- **GIVEN** a dynamic model is registered and listed by `/v1/models`
- **AND** the `/v1/chat/completions` route contains a sealed dynamic binding for
  its alias
- **WHEN** a client requests a chat completion with that alias
- **THEN** the request SHALL pass route model authorization

#### Scenario: Unknown model returns 404

- **WHEN** a chat completion request names a model the host cannot load
- **THEN** `guest-openai` responds with HTTP `404` and an OpenAI-shaped
  `model_not_found` error body

#### Scenario: Present without feature injection

- **GIVEN** a node whose sealed manifest declares the `guest-openai` user routes
- **WHEN** the topology graph is built from that manifest
- **THEN** `guest-openai` appears as a user route regardless of whether the
  binary was compiled with `ai-inference`
