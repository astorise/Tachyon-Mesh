## MODIFIED Requirements

### Requirement: User-space OpenAI-compatible FaaS

The OpenAI-compatible HTTP surface SHALL be provided by a single **user-role** FaaS (`guest-openai`) built against the `faas-guest` WIT world. It SHALL expose `GET /ai/v1/models` and `POST /ai/v1/chat/completions`, and it SHALL NOT expose the former public `/v1/models` or `/v1/chat/completions` routes. It SHALL NOT be a system FaaS injected by a compile-time feature flag.

#### Scenario: Model listing returns OpenAI-compatible shape

- **GIVEN** the registry contains at least one available model
- **WHEN** a client requests `GET /ai/v1/models`
- **THEN** `guest-openai` returns a JSON body with `object: "list"` and a `data` array
- **AND** each item includes an `id`, `object: "model"`, and `owned_by: "tachyon-mesh"`

#### Scenario: Chat completions runs real inference

- **GIVEN** a sealed model alias the route is allowed to use
- **WHEN** a client requests `POST /ai/v1/chat/completions` naming that model
- **THEN** `guest-openai` loads the model on the CPU accelerator, hands the structured conversation and sampling parameters to the host, and returns an OpenAI-shaped `chat.completion` whose assistant message carries the generated text

#### Scenario: Former public routes are absent

- **WHEN** a client requests `/v1/models` or `/v1/chat/completions`
- **THEN** the node returns `404` because those routes are not sealed

#### Scenario: Unknown model returns 404

- **WHEN** a chat completion request names a model the host cannot load
- **THEN** `guest-openai` responds with HTTP `404` and an OpenAI-shaped `model_not_found` error body

#### Scenario: Present without feature injection

- **GIVEN** a node whose sealed manifest declares the `guest-openai` user routes
- **WHEN** the topology graph is built from that manifest
- **THEN** `guest-openai` appears as a user route regardless of whether the binary was compiled with `ai-inference`

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

### Requirement: Upload notification persists to the shared registry

`system-faas-model-broker` SHALL notify `guest-openai` over HTTP when a newly uploaded model becomes available, targeting the internal register endpoint, so the model is recorded in the shared `ai-models-registry` table and becomes listable.

#### Scenario: Broker upload makes a model listable

- **WHEN** `model-broker` completes a model upload and notifies `guest-openai`'s register endpoint
- **THEN** the model is written to `ai-models-registry`
- **AND** a subsequent `GET /ai/v1/models` includes that model
