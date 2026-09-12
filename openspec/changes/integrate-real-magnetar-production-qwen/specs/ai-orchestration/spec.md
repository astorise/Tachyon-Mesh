## MODIFIED Requirements

### Requirement: AI Orchestration Panel
The Tachyon UI shell SHALL expose a `<tachyon-ai-panel>` web component for configuring runtime-backed AI manifest fields through the shared dashboard base. The AI Orchestration view SHALL also host the `<tachyon-model-upload-panel>` control for uploading model files (see `ai-model-upload-ui`).

#### Scenario: Operator adjusts KV cache
- **WHEN** the operator moves the KV cache slider
- **THEN** the panel updates the visible cache value immediately without a backend round trip

#### Scenario: Operator applies AI KV-cache configuration
- **WHEN** the operator submits the AI panel with available model aliases
- **THEN** the panel reads the active manifest through `get_manifest_config`
- **AND** writes one `kv_caches` entry per available model alias without overwriting unrelated cache entries
- **AND** applies the updated manifest through `apply_manifest_config`
- **AND** the panel shows a feedback message explaining that experimental model controls are not runtime manifest fields until a matching `IntegrityConfig` field exists

#### Scenario: AI view exposes the model-upload panel
- **WHEN** the AI Orchestration view is rendered (with `has_ai` true)
- **THEN** the `<tachyon-model-upload-panel>` control is present for uploading a model file

## REMOVED Requirements

### Requirement: VRAM Priority Tiers
**Reason**: Local safetensors layer residency is no longer an active Tachyon execution contract after the Magnetar cutover. Magnetar owns Provider memory management for production Qwen execution.
**Migration**: Keep placement and QoS policy in Tachyon; expose Magnetar-owned memory capabilities through Magnetar Provider contracts when needed.

### Requirement: Predictive Broker Prewarms Tenant LoRA
**Reason**: Tenant LoRA prewarm was tied to the historical local adapter path and would reintroduce an active LoRA execution promise that Magnetar integration does not provide.
**Migration**: Reintroduce adapter prewarm only through explicit Magnetar Component/Provider support in a future change.
