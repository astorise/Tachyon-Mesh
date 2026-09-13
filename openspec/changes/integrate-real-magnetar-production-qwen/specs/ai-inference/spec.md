## ADDED Requirements

### Requirement: Tachyon MUST delegate local inference through opaque Magnetar Components
For local inference bindings, the host SHALL delegate execution to a Magnetar
inference Component. Tachyon SHALL pass only Component source/provenance,
host-controlled trust policy, generic placement constraints, QoS/deadline
policy, invocation payload bytes, and streaming transport. Tachyon SHALL NOT
know the Component's model family, model format, tokenizer, chat template,
model architecture, concrete Provider implementation, or model instance type.

#### Scenario: Component artifact loads through Magnetar
- **WHEN** a binding points at a Tachyon-staged `magnetar:` Component artifact
  directory
- **THEN** Tachyon resolves exactly one explicit `*.component.wasm` artifact
  and its Magnetar Component manifest from that directory
- **AND** Tachyon hands the Component artifact bytes, authorized source root,
  trust policy, and placement constraints to Magnetar
- **AND** Magnetar owns production ingestion, tokenizer loading, manifest
  normalization, model instance materialization, execution planning, and
  Provider execution behind the Component API
- **AND** Tachyon does not parse model payload bytes, tokenizer metadata, model
  architecture tensors, or concrete Provider artifacts itself

#### Scenario: Missing Component artifact fails closed
- **WHEN** a `magnetar:` binding root does not contain an explicit
  `*.component.wasm` artifact and matching Component manifest
- **THEN** Tachyon rejects the binding
- **AND** neither Tachyon nor the generic Magnetar adapter selects a compiled-in
  default inference Component

#### Scenario: Untrusted Component artifact fails before materialization
- **WHEN** Magnetar inspects a Component artifact whose manifest is not trusted
  by Tachyon's host-controlled trust policy
- **THEN** loading fails before Provider resource materialization
- **AND** Tachyon reports a trust-shaped local inference error

#### Scenario: Artifact-local trust policy is ignored
- **WHEN** a Tachyon-staged artifact contains trust metadata that trusts its own
  manifest digest
- **THEN** Tachyon still treats the artifact as untrusted unless the
  host-controlled trust store outside the artifact root trusts that digest
- **AND** artifacts cannot self-authorize by shipping trust metadata inside the
  bundle

### Requirement: Tachyon MUST NOT depend on model loaders or concrete Providers
The host SHALL use Magnetar's generic inference Component adapter and runtime
contracts. `core-host` SHALL NOT depend directly on Magnetar model loader crates
or concrete Provider implementation crates. CUDA selection SHALL be expressed as
a generic placement constraint and resolved behind Magnetar's Component API.

#### Scenario: CPU policy invokes a Component
- **WHEN** route policy selects CPU for an admitted local Component
- **THEN** Tachyon invokes the Component through Magnetar
- **AND** the concrete model loader, tokenizer, model instance, and Provider
  remain Magnetar-owned

#### Scenario: CUDA policy fails closed without direct Provider ownership
- **WHEN** route policy explicitly selects CUDA for an admitted local Component
- **THEN** Tachyon forwards a CUDA placement constraint to Magnetar
- **AND** a successful response cannot have silently used CPU as a fallback
- **AND** Tachyon core does not construct or import a concrete CUDA Provider

#### Scenario: Dynamic loading is target-aware
- **WHEN** a dynamic `magnetar:` Component alias is requested for a specific
  accelerator
- **THEN** Tachyon lazy-loads the binding for that requested accelerator
- **AND** a CPU-loaded Component cannot satisfy a GPU request
- **AND** an already-loaded GPU Component for another alias cannot make a
  dynamic GPU request fall back to CPU

### Requirement: Model and protocol controls MUST stay outside Tachyon core
The Tachyon core SHALL NOT translate tool-call dialects, structured-output
schemas, tokenizer/chat-template behavior, or model-specific prompt formats.
Those controls SHALL be handled or rejected by `guest-openai`, the Component, or
Magnetar.

#### Scenario: Tool-call controls are not prompt-mutated by Tachyon core
- **WHEN** a local invocation includes `tools`, `tool_choice`, or
  `tool_call_parser`
- **THEN** Tachyon core does not rewrite those controls into model prompts
- **AND** the responsible guest/Component boundary handles the dialect or
  rejects the invocation as invalid

#### Scenario: Structured output is guaranteed or rejected
- **WHEN** a local invocation asks for structured JSON output
- **THEN** Tachyon does not treat a plain text instruction as a guarantee
- **AND** the responsible inference layer either validates/guarantees the JSON
  object or rejects the invocation as invalid

#### Scenario: Deadlines remain generic execution policy
- **WHEN** a local invocation supplies a generation deadline
- **THEN** Tachyon forwards it as generic execution policy
- **AND** Magnetar enforces cancellation in the Component execution path

### Requirement: Tachyon streaming MUST consume Magnetar Component events
For local Magnetar Component streaming, Tachyon SHALL consume Magnetar streaming
events and relay deltas. Tachyon SHALL NOT buffer a full generation and then
emit it as a fabricated streaming chunk.

#### Scenario: Streaming emits token deltas from Magnetar
- **WHEN** a local request uses streaming
- **THEN** Tachyon obtains token deltas from Magnetar streaming events
- **AND** cancellation from the downstream stream propagates with
  `ControlFlow::Break`
- **AND** Magnetar execution stops instead of completing the full requested
  token budget after the downstream stream disconnects

### Requirement: Active WIT inference surface MUST stay Component-scoped
The public `wit/ai/inference.wit` contract SHALL expose only the local
inference request/response function needed by Tachyon's Magnetar-backed
Component runtime. Historical LoRA adapter injection, per-call layer-wise memory
profiles, handle-based layer execution, and local multi-device topology
validation SHALL NOT remain in the active inference WIT package.

#### Scenario: Historical local execution contracts are absent
- **WHEN** `wit/ai/inference.wit` is inspected
- **THEN** it does not expose `adapter-id`, `infer-with-options`,
  `memory-profile`, `layer-execution`, or `parallel-execution`
- **AND** model format parsing, layer execution, tensor handles, and Provider
  execution remain Magnetar-owned

### Requirement: Tachyon MUST NOT fabricate Magnetar execution or hardware identities
The host SHALL consume Magnetar runtime, Provider, and Device contracts rather
than creating local stand-ins for Magnetar handles or capability
advertisements. Tachyon SHALL NOT define permanent local kernel/tensor IDs,
hardcoded device identities, hardcoded VRAM/dtype support, environment-variable
hardware truth, or pseudo Magnetar memory residency as the source of mesh
routing truth.

#### Scenario: Capability advertisement comes from Magnetar metadata
- **WHEN** Tachyon advertises local AI execution capacity to the mesh
- **THEN** the advertised Provider and Device data is derived from Magnetar
  metadata
- **AND** it is not derived from hardcoded Tachyon constants

#### Scenario: Tachyon does not expose fake Magnetar handles
- **WHEN** the host invokes an inference Component through Magnetar
- **THEN** Tachyon does not mint local kernel or tensor handles to represent
  Magnetar-owned execution resources
- **AND** Tensor Resource identity and Provider execution remain inside Magnetar

## REMOVED Requirements

### Requirement: Candle LLM prefill is chunked and configurable
**Reason**: Production local inference is now delegated to Magnetar Components rather than Tachyon's Candle LLM runtime.
**Migration**: Use Magnetar Component execution policy. Future prefill chunking requirements belong in the Component/Magnetar contract or a Tachyon policy layer that does not inspect model tensors.

### Requirement: Downstream Candle quantization kernels MUST be consumed through the pinned fork
**Reason**: Candle is no longer the active local inference dependency for the Magnetar cutover path.
**Migration**: Quantization support must be represented through Magnetar Component/provider capabilities when exposed.

### Requirement: Candle engine hot-swaps adapter weights and bounds context-switching overhead
**Reason**: Adapter execution through a Candle engine is not part of the real Magnetar Component cutover.
**Migration**: LoRA and adapter multiplexing must be re-specified against Magnetar Components and Providers before being re-enabled for local production inference.

### Requirement: `candle-cuda` MUST be documented as the single CUDA switch
**Reason**: CUDA coverage for the active local inference path must use Magnetar Component/CUDA gates, not Candle feature switches.
**Migration**: Use Magnetar CUDA CI checks and explicit fail-closed placement requirements.

### Requirement: WIT layer-wise and multi-device local execution contracts
**Reason**: Layer-wise tensor handles and topology validation were historical Tachyon-local execution contracts. Active local execution now belongs behind Magnetar Component, model instance, prepared execution plan, memory manager, and Provider APIs.
**Migration**: Keep routing and placement policy in Tachyon; express model execution topology through Magnetar contracts when those capabilities are exposed.

### Requirement: Per-call LoRA adapter injection in local inference WIT
**Reason**: Per-call adapter overlays are not part of the current Magnetar Component integration and must not stay as a public local inference promise.
**Migration**: Reintroduce adapter behavior only through explicit Magnetar Component/Provider support in a future change.
