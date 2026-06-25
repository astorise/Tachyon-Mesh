# ai-orchestration Specification

## Purpose
Define the Tachyon UI controls and backend validation contract for AI orchestration, accelerator selection, and KV cache configuration.
## Requirements
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

### Requirement: VRAM Priority Tiers
The AI inference host SHALL assign each safetensors layer residency a VRAM priority of `Active`, `Hot`, or `Volatile`.

#### Scenario: Volatile prewarm memory is reclaimed for live inference
- **GIVEN** a predictive LoRA adapter is resident in VRAM with `Volatile` priority
- **WHEN** a live inference request needs VRAM for `Active` tensors
- **THEN** the host SHALL evict the volatile residency before failing the live request for insufficient VRAM
- **AND** the live request SHALL keep its `Active` residency protected from speculative evictions

#### Scenario: Expired hot memory is reclaimed after volatile memory
- **GIVEN** both `Volatile` and expired `Hot` safetensors allocations are resident in VRAM
- **WHEN** reclaiming volatile memory alone is insufficient for an active request
- **THEN** the host SHALL reclaim expired `Hot` allocations before returning an out-of-memory error

### Requirement: Predictive Broker Prewarms Tenant LoRA
The model broker SHALL translate auth session CDC events into volatile layer prewarm instructions for the tenant default LoRA adapter.

#### Scenario: Auth session creation schedules volatile prewarm
- **GIVEN** the broker receives an auth session mutation event with operation `insert`
- **AND** the event payload contains a tenant identifier
- **WHEN** the event belongs to the auth session namespace
- **THEN** the broker SHALL resolve the tenant default LoRA model id
- **AND** return a layer load instruction with priority `volatile`

#### Scenario: Non-auth mutations are ignored
- **GIVEN** the broker receives a mutation event outside the auth namespace
- **WHEN** evaluating predictive prewarm eligibility
- **THEN** the broker SHALL NOT produce a layer load instruction

### Requirement: Dynamic VRAM TTL From Time-Series Heuristics
The model broker SHALL calculate a dynamic volatile VRAM TTL from tenant prompt-history density for the current hour.

#### Scenario: High follow-up probability extends volatile TTL
- **GIVEN** tenant prompt history shows a follow-up probability greater than `0.8`
- **WHEN** a prompt finishes
- **THEN** the broker SHALL select a volatile VRAM TTL of `1800` seconds

#### Scenario: Standard follow-up probability keeps default TTL
- **GIVEN** tenant prompt history shows a follow-up probability less than or equal to `0.8`
- **WHEN** a prompt finishes
- **THEN** the broker SHALL select the standard volatile VRAM TTL of `300` seconds

### Requirement: AI model registry WIT contract
Tachyon Mesh SHALL define a `wit/ai/model-registry.wit` contract that exposes a `list-models` function returning model alias, engine, VRAM requirement, and status metadata for locally available inference models.

#### Scenario: Registry exposes available models
- **GIVEN** a system FaaS needs the local model inventory
- **WHEN** it calls the model registry `list-models` function
- **THEN** it receives a list of model records with alias, engine, VRAM requirement, and status fields

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

