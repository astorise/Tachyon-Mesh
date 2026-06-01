## MODIFIED Requirements

### Requirement: OpenAI adapter model listing

The `guest-openai` user FaaS SHALL serve `/v1/models` by reading the `ai-models-registry` `kv-partition` table directly and transforming each Tachyon model record into an OpenAI-compatible model object. It SHALL NOT call a separate registry FaaS to obtain the model list.

#### Scenario: Client lists OpenAI-compatible models

- **GIVEN** the `ai-models-registry` table contains at least one available model
- **WHEN** an authenticated client requests `/v1/models`
- **THEN** `guest-openai` returns an OpenAI-compatible JSON response with `object: "list"` and a `data` array
- **AND** each item includes an `id`, `object: "model"`, and `owned_by: "tachyon-mesh"`

## REMOVED Requirements

### Requirement: Gateway routes OpenAI-compatible endpoints to the adapter

**Reason**: `/v1/models` and `/v1/chat/completions` are now sealed **user** routes that resolve directly to the `guest-openai` FaaS through the normal route registry. The system gateway no longer needs to dispatch these paths to a system adapter.

**Migration**: Declare the `guest-openai` user routes in the deployment manifest (e.g. `examples/guest-examples/manifest.json`). Requests to `/v1/*` are matched and dispatched by the host route registry; remove any reliance on `system-faas-gateway` forwarding `/v1/*` to `system-faas-openai-adapter`.
