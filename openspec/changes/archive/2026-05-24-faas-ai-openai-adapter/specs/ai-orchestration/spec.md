# ai-orchestration Delta

## ADDED Requirements

### Requirement: AI model registry WIT contract
Tachyon Mesh SHALL define a `wit/ai/model-registry.wit` contract that exposes a `list-models` function returning model alias, engine, VRAM requirement, and status metadata for locally available inference models.

#### Scenario: Registry exposes available models
- **GIVEN** a system FaaS needs the local model inventory
- **WHEN** it calls the model registry `list-models` function
- **THEN** it receives a list of model records with alias, engine, VRAM requirement, and status fields

### Requirement: OpenAI adapter model listing
The `system-faas-openai-adapter` FaaS SHALL serve `/v1/models` by calling the AI model registry and transforming each Tachyon model record into an OpenAI-compatible model object.

#### Scenario: Client lists OpenAI-compatible models
- **GIVEN** the registry contains at least one available model
- **WHEN** an authenticated client requests `/v1/models`
- **THEN** the adapter returns an OpenAI-compatible JSON response with `object: "list"` and a `data` array
- **AND** each item includes an `id`, `object: "model"`, and `owned_by: "tachyon-mesh"`

### Requirement: OpenAI adapter scope enforcement
The OpenAI-compatible adapter SHALL require the requesting identity to have the `ai:model:read` scope before serving model registry data.

#### Scenario: Missing scope is rejected
- **GIVEN** a client identity lacks the `ai:model:read` scope
- **WHEN** it requests `/v1/models`
- **THEN** the adapter rejects the request without exposing model metadata

### Requirement: Gateway routes OpenAI-compatible endpoints to the adapter
The system FaaS gateway SHALL route `/v1/models` and `/v1/chat/completions` requests to `system-faas-openai-adapter` while preserving HTTP headers.

#### Scenario: Gateway forwards OpenAI-compatible request
- **WHEN** a request reaches the gateway for `/v1/chat/completions`
- **THEN** the gateway dispatches it to `system-faas-openai-adapter`
- **AND** authorization and tracing headers remain available to the adapter
