# openai-compatible-faas Specification

## Purpose
TBD - created by archiving change faas-openai-user-example. Update Purpose after archive.
## Requirements
### Requirement: User-space OpenAI-compatible FaaS

The OpenAI-compatible HTTP surface SHALL be provided by a single **user-role** FaaS (`guest-openai`) built against the `faas-guest` WIT world. It SHALL expose `GET /v1/models` and `POST /v1/chat/completions`, and it SHALL NOT be a system FaaS injected by a compile-time feature flag.

#### Scenario: Model listing returns OpenAI-compatible shape

- **GIVEN** the registry contains at least one available model
- **WHEN** a client requests `GET /v1/models`
- **THEN** `guest-openai` returns a JSON body with `object: "list"` and a `data` array
- **AND** each item includes an `id`, `object: "model"`, and `owned_by: "tachyon-mesh"`

#### Scenario: Chat completions is a stub

- **WHEN** a client requests `POST /v1/chat/completions`
- **THEN** `guest-openai` responds with HTTP `501` and an OpenAI-shaped error body

#### Scenario: Present without feature injection

- **GIVEN** a node whose sealed manifest declares the `guest-openai` user routes
- **WHEN** the topology graph is built from that manifest
- **THEN** `guest-openai` appears as a user route regardless of whether the binary was compiled with `ai-inference`

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
- **WHEN** `GET /v1/models` is served by the same instance immediately afterward
- **THEN** the response includes the newly registered model

#### Scenario: Visibility across instances

- **GIVEN** a model is registered on instance A
- **WHEN** `GET /v1/models` is served by a different instance B
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

`system-faas-model-broker` SHALL notify `guest-openai` over HTTP when a newly uploaded model becomes available, targeting the register endpoint, so the model is recorded in the shared `ai-models-registry` table and becomes listable.

#### Scenario: Broker upload makes a model listable

- **WHEN** `model-broker` completes a model upload and notifies `guest-openai`'s register endpoint
- **THEN** the model is written to `ai-models-registry`
- **AND** a subsequent `GET /v1/models` includes that model

