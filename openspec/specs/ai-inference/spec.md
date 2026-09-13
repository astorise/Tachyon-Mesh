# ai-inference Specification

## Purpose
Defines Tachyon's active AI inference contract after the Magnetar cutover.
Tachyon owns Component artifact transport, integrity/trust policy, mesh routing,
QoS, placement constraints, admission, deadlines, and streaming transport.
Magnetar owns model format handling, tokenizer/chat template handling, model
instance lifecycle, execution planning, Provider/Device selection, kernel
execution, generation, and streaming semantics behind the Component boundary.

## Requirements
### Requirement: Local inference is delegated through opaque Magnetar Components
For local inference bindings, Tachyon SHALL delegate execution to a Magnetar
inference Component. Tachyon SHALL treat the invocation payload as opaque except
for mesh-owned controls such as route identity, trust policy, placement, QoS,
deadline, and resource admission. Tachyon SHALL NOT know the model family,
model format, tokenizer, chat template, model architecture, Provider
implementation, or model instance type carried by the Component.

#### Scenario: Tachyon loads a local inference Component
- **WHEN** a binding points at a `magnetar:` Component artifact directory
- **THEN** Tachyon passes the authorized Component source, placement
  constraints, and host-controlled trust policy to Magnetar
- **AND** Magnetar owns any production ingestion, tokenizer loading, manifest
  normalization, model instance materialization, execution planning, and
  Provider execution needed by that Component
- **AND** Tachyon does not parse model payloads, tokenizer files, model
  architecture metadata, or Provider-specific artifacts itself

#### Scenario: Magnetar trust policy is explicit
- **WHEN** Magnetar inspects a Component artifact whose manifest digest is not
  trusted by Tachyon's host-controlled trust policy outside the artifact root
- **THEN** Tachyon rejects the binding before execution
- **AND** parsing success, source kind, and local filesystem authorization do
  not grant trust
- **AND** trust metadata shipped inside the artifact cannot self-authorize the
  bundle

#### Scenario: Placement constraints remain generic
- **WHEN** route policy selects CPU or CUDA placement for a local Component
- **THEN** Tachyon forwards that placement constraint to Magnetar
- **AND** Magnetar resolves the concrete Provider/Device behind its Component
  API
- **AND** explicit CUDA placement fails closed when Magnetar reports CUDA
  unavailable instead of silently falling back to CPU

#### Scenario: Dynamic loading is target-aware
- **WHEN** a dynamic `magnetar:` Component alias is requested for a specific
  accelerator
- **THEN** Tachyon lazy-loads the binding for that requested accelerator
- **AND** a CPU-loaded Component cannot satisfy a GPU request
- **AND** an already-loaded GPU Component for another alias cannot make a
  dynamic GPU request fall back to CPU

#### Scenario: Request semantics stay beyond the Tachyon core boundary
- **WHEN** a local invocation payload contains model- or protocol-specific
  controls such as tool-call dialects or structured-output guarantees
- **THEN** Tachyon core does not translate those controls into model prompts
- **AND** the responsible guest, Component, or Magnetar layer either handles the
  control or rejects it as an invalid request
- **AND** Tachyon does not advertise structured-output capability unless the
  responsible inference layer guarantees or validates the result

#### Scenario: Streaming is delegated to Magnetar Component events
- **WHEN** a local request uses streaming
- **THEN** Tachyon relays token/content deltas produced by Magnetar
- **AND** downstream streaming cancellation propagates to Magnetar so execution
  does not continue to the full requested budget after disconnect
- **AND** Tachyon does not fabricate streaming chunks from a buffered full
  response

### Requirement: Active WIT inference surface remains Component-scoped
The public `wit/ai/inference.wit` contract SHALL expose only the local
inference request/response function needed by Tachyon's Magnetar-backed
Component runtime. Historical LoRA adapter injection, per-call layer-wise memory
profiles, handle-based layer execution, local multi-device topology validation,
and model-format-specific controls SHALL NOT remain in the active inference WIT
package.

#### Scenario: Historical local execution contracts are absent
- **WHEN** `wit/ai/inference.wit` is inspected
- **THEN** it does not expose `adapter-id`, `infer-with-options`,
  `memory-profile`, `layer-execution`, or `parallel-execution`
- **AND** model format parsing, layer execution, tensor handles, and Provider
  execution remain Magnetar-owned

### Requirement: Local legacy text-generation compatibility is not an active inference surface
Tachyon SHALL NOT expose Cargo feature aliases, runtime modules, CI gates, or
canonical requirements that select a local legacy text-generation path.
Historical model-family-specific bindings and compatibility directories SHALL
be rejected unless they satisfy the active Magnetar Component contract.

#### Scenario: Old local text-generation feature names are unavailable
- **WHEN** a developer attempts to enable an obsolete local text-generation
  compatibility feature
- **THEN** Cargo reports that the feature does not exist
- **AND** the build does not compile a hidden local text-generation fallback

#### Scenario: Non-Magnetar local model directories are rejected
- **WHEN** a local model-style directory is configured without the `magnetar:`
  Component binding prefix
- **THEN** Tachyon rejects the binding with a typed unsupported-binding error
- **AND** it does not route the request to a legacy local runtime

#### Scenario: GPU proof cannot pass vacuously
- **WHEN** CI validates GPU inference behavior
- **THEN** the selected test set must contain at least one Magnetar CUDA test
- **AND** a zero-test selection is a CI failure

### Requirement: Magnetar local inference dependencies remain feature-gated
The `core-host` runtime SHALL keep Magnetar-backed local inference dependencies
behind the `ai-inference` Cargo feature without preserving historical local
inference compatibility as an active product contract.

#### Scenario: Default host builds without AI inference
- **WHEN** a developer builds `core-host` without enabling `ai-inference`
- **THEN** the host compiles successfully without local production inference
  dependencies
- **AND** the default release and container workflows remain unchanged

#### Scenario: AI inference build links Magnetar Component API
- **WHEN** a developer builds `core-host` with `--features ai-inference`
- **THEN** the host links the Magnetar runtime and generic inference Component
  adapter
- **AND** it does not depend directly on concrete Magnetar model loaders or
  concrete Provider implementation crates

#### Scenario: AI guest runs without ai-inference feature
- **WHEN** `core-host` is built without `--features ai-inference`
- **AND** an AI guest or route requires a local Component binding
- **THEN** execution fails gracefully with an error naming the missing
  `ai-inference` feature
