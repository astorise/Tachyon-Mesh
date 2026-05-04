# tauri-configurator Specification

## ADDED Requirements

### Requirement: The UI MUST provide declarative builders for AI Multiplexing
The frontend SHALL expose a dedicated "AI Orchestration" dashboard that allows operators to visually map Layer 7 request headers to specific LoRA adapter assets without manual JSON editing.

#### Scenario: Configuring a new tenant-specific LLM adapter
- **WHEN** the user navigates to the AI Orchestration view and adds a "Routing Rule" mapping `X-Tenant: HR` to `hr-lora.gguf`
- **THEN** the Vanilla JS controller constructs the `sharing_strategy` JSON block
- **AND** submits the payload, allowing the Edge node to hot-swap the HR adapter into VRAM instantly upon receiving matching traffic.
