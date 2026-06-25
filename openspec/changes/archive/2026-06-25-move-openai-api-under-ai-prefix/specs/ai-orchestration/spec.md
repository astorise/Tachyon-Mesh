## MODIFIED Requirements

### Requirement: OpenAI adapter model listing

The `guest-openai` user FaaS SHALL serve `/ai/v1/models` by reading the `ai-models-registry` `kv-partition` table directly and transforming each Tachyon model record into an OpenAI-compatible model object. It SHALL NOT call a separate registry FaaS to obtain the model list.

#### Scenario: Client lists OpenAI-compatible models

- **GIVEN** the `ai-models-registry` table contains at least one available model
- **WHEN** an authenticated client requests `/ai/v1/models`
- **THEN** `guest-openai` returns an OpenAI-compatible JSON response with `object: "list"` and a `data` array
- **AND** each item includes an `id`, `object: "model"`, and `owned_by: "tachyon-mesh"`

### Requirement: OpenAI adapter scope enforcement

The OpenAI-compatible adapter SHALL require the requesting identity to have the `ai:model:read` scope before serving model registry data.

#### Scenario: Missing scope is rejected

- **GIVEN** a client identity lacks the `ai:model:read` scope
- **WHEN** it requests `/ai/v1/models`
- **THEN** the adapter rejects the request without exposing model metadata
