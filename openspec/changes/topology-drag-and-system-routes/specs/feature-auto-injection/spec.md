## MODIFIED Requirements

### Requirement: ai-inference injects three AI system routes
When built with `--features ai-inference`, the system SHALL inject
`/system/model-broker`, `/system/ai-list-model`, **and**
`/system/ai-openai-adapter` into every `IntegrityConfig` at startup and hot-reload.

#### Scenario: ai-openai-adapter appears in topology for ai-inference build
- **WHEN** `core-host` is built with `--features ai-inference`
- **THEN** `GET /admin/nodes` reports `ai-openai-adapter` in `active_systems`
- **THEN** the topology canvas shows a `system-faas` node labelled `ai-openai-adapter`
