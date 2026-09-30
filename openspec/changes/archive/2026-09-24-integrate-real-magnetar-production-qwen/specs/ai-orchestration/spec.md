## MODIFIED Requirements

### Requirement: AI Orchestration Panel
The Tachyon UI shell SHALL expose a `<tachyon-ai-panel>` web component for configuring runtime-backed AI manifest fields through the shared dashboard base. The AI Orchestration view SHALL host artifact upload controls for staging inference Component artifacts and component payload bundles without making Tachyon parse or classify component formats.

#### Scenario: Operator applies AI KV-cache configuration
- **WHEN** the operator submits the AI panel with available Component aliases
- **THEN** the panel reads the active manifest through `get_manifest_config`
- **AND** writes one `kv_caches` entry per available Component alias without overwriting unrelated cache entries
- **AND** applies the updated manifest through `apply_manifest_config`
- **AND** the panel shows feedback that execution-specific component controls belong to the selected Component or Magnetar contract, not to Tachyon runtime manifest fields

#### Scenario: AI view exposes artifact upload
- **WHEN** the AI Orchestration view is rendered with AI support enabled
- **THEN** it exposes controls for uploading or selecting Component/artifact bundles

### Requirement: AI Panel Placement Bindings
The Tachyon AI panel SHALL expose each route inference Component binding from the active manifest and let operators configure only generic placement fields that Tachyon owns.

#### Scenario: Operator lists manifest Component placement bindings
- **WHEN** the AI panel loads
- **THEN** it reads the active manifest through `get_manifest_config`
- **AND** lists every `routes[].inference_components[]` binding that has an alias
- **AND** it shows the cluster accelerator summary from `get_cluster_hardware_summary` as placement context

#### Scenario: Operator applies Component placement
- **WHEN** the operator edits a Component binding's accelerator placement, device IDs, QoS, or memory budget
- **THEN** the panel mutates only Tachyon-owned placement fields on that `routes[].inference_components[]` entry
- **AND** it applies the updated manifest through `apply_manifest_config`
- **AND** it does not write component-format, tokenizer, layer topology, speculative decode, CUDA graph, or Provider-specific execution fields

### Requirement: Component registry WIT contract
Tachyon Mesh SHALL define Component/artifact registry contracts that expose alias, engine, placement, availability, and status metadata for locally available inference Components without exposing Component-native loading or generation APIs from core.

#### Scenario: Registry exposes available Components
- **GIVEN** a guest needs the local inference inventory
- **WHEN** it reads the shared Component registry
- **THEN** it receives Component records with alias, engine, resource requirement, status, and artifact path metadata

### Requirement: OpenAI artifact listing
The `guest-openai` user FaaS SHALL serve `/ai/v1/components` by reading the `ai-components-registry` `kv-partition` table directly and transforming each Tachyon Component record into an OpenAI-compatible component object. It SHALL NOT call a separate core Component registry FaaS.

#### Scenario: Client lists OpenAI-compatible components
- **GIVEN** the `ai-components-registry` table contains at least one available Component
- **WHEN** an authenticated client requests `/ai/v1/components`
- **THEN** `guest-openai` returns an OpenAI-compatible JSON response with `object: "list"` and a `data` array
- **AND** each item includes an `id`, `object: "component"`, and `owned_by: "tachyon-mesh"`

## REMOVED Requirements

### Requirement: VRAM Priority Tiers
**Reason**: Local component layer residency is no longer an active Tachyon execution contract after the Magnetar cutover. Magnetar owns Provider memory management for local Component execution.
**Migration**: Keep placement and QoS policy in Tachyon; expose Magnetar-owned memory capabilities through Magnetar Provider contracts when needed.

### Requirement: Predictive Broker Prewarms Tenant Component training
**Reason**: Tenant Component training prewarm was tied to the historical local artifact path and would reintroduce an active artifact execution promise that Magnetar integration does not provide.
**Migration**: Reintroduce artifact prewarm only through explicit Magnetar Component/Provider support in a future change.

### Requirement: AI Component registry WIT contract
**Reason**: Core WIT must not expose a Component-native registry after the Component boundary closure.
**Migration**: Use Component/artifact registry metadata and let `guest-openai` project those records into OpenAI-compatible `/components` responses.
