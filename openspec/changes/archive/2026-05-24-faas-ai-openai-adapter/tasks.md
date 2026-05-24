# Implementation Tasks

- [x] **Task 1: Define AI WIT Contract**
  - Create `wit/ai/model-registry.wit`.
  - Compile the bindings to allow internal communication between the Registry FaaS and the Adapter FaaS.

- [x] **Task 2: Build `system-faas-ai-list-model`**
  - Implement the Registry FaaS. It must subscribe to the `model-broker` lifecycle events to maintain its internal RedDB K/V registry.
  - Implement the `list-models()` WIT export.

- [x] **Task 3: Build `system-faas-openai-adapter`**
  - Implement the REST API server.
  - Apply the requested security scopes (require that the requesting entity has `ai:model:read` scope).
  - Add logic to map Tachyon internal model names to OpenAI-compatible IDs if aliases are required.

- [x] **Task 4: Gateway Routing**
  - Update `system-faas-gateway` routing to map `/v1/models` and `/v1/chat/completions` requests to the `openai-adapter` FaaS, ensuring full HTTP header transparency.
