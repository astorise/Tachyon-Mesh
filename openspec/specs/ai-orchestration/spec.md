# ai-orchestration Specification

## Purpose
Define the Tachyon UI controls and backend validation contract for AI orchestration, accelerator selection, and KV cache configuration.
## Requirements
### Requirement: AI Orchestration Panel
The Tachyon UI shell SHALL expose a `<tachyon-ai-panel>` web component for configuring LoRA multiplexing, edge KV cache size, and encrypted TDE key material through the shared dashboard base.

#### Scenario: Operator adjusts KV cache
- **WHEN** the operator moves the KV cache slider
- **THEN** the panel updates the visible cache value immediately without a backend round trip

#### Scenario: Operator applies AI configuration
- **WHEN** the operator submits the AI panel
- **THEN** the panel sends a `config-ai` payload with `lora_mode`, `kv_cache_size`, and `tde_key` to `apply_configuration`
- **AND** the panel shows the backend validation result in its feedback zone

### Requirement: Hardware Accelerator Panel
The Tachyon UI shell SHALL expose a `<tachyon-hardware-panel>` web component for selecting NPU, TPU, or GPU acceleration and enabling eBPF XDP offloading.

#### Scenario: Operator applies accelerator policy
- **WHEN** the operator selects an accelerator and toggles XDP offload
- **THEN** the panel sends those choices as part of a `config-ai` payload for strict backend validation

### Requirement: AI Payload Validation
The Tauri backend SHALL validate `config-ai` payloads against a strict Serde contract that rejects unknown fields and invalid ranges.

#### Scenario: Valid AI payload
- **WHEN** the backend receives a `config-ai` payload with a supported LoRA mode, a KV cache size from 8 to 128 GB, and a non-empty TDE key
- **THEN** it returns a successful validation response

#### Scenario: Invalid AI payload
- **WHEN** the backend receives a `config-ai` payload with unknown fields, an unsupported mode, an empty TDE key, or a KV cache size outside the allowed range
- **THEN** it returns a failed validation response describing the invalid field
