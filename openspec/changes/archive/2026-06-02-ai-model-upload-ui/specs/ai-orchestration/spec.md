## MODIFIED Requirements

### Requirement: AI Orchestration Panel
The Tachyon UI shell SHALL expose a `<tachyon-ai-panel>` web component for configuring LoRA multiplexing, edge KV cache size, and encrypted TDE key material through the shared dashboard base. The AI Orchestration view SHALL also host the `<tachyon-model-upload-panel>` control for uploading model files (see `ai-model-upload-ui`).

#### Scenario: Operator adjusts KV cache
- **WHEN** the operator moves the KV cache slider
- **THEN** the panel updates the visible cache value immediately without a backend round trip

#### Scenario: Operator applies AI configuration
- **WHEN** the operator submits the AI panel
- **THEN** the panel sends a `config-ai` payload with `lora_mode`, `kv_cache_size`, and `tde_key` to `apply_configuration`
- **AND** the panel shows the backend validation result in its feedback zone

#### Scenario: AI view exposes the model-upload panel
- **WHEN** the AI Orchestration view is rendered (with `has_ai` true)
- **THEN** the `<tachyon-model-upload-panel>` control is present for uploading a model file
