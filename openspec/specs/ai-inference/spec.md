# ai-inference Specification

## Purpose
Defines Tachyon's active AI inference contract after the Magnetar cutover.
Tachyon owns artifact transport, mesh routing, QoS, placement policy, and
admission. Magnetar owns production model ingestion, tokenizer loading, model
manifest normalization, model instance construction, execution planning, and
Provider execution.

## Requirements
### Requirement: Magnetar production Qwen is the active local text-generation runtime
For local Qwen text-generation bindings, Tachyon SHALL delegate production model
ingestion, tokenizer loading, manifest normalization, model instance
construction, execution planning, and Provider execution to Magnetar. Tachyon
SHALL NOT parse model weight payloads, convert model dtypes, construct a Qwen
graph, or fabricate Magnetar tensor, kernel, hardware, or capability identities.

#### Scenario: Tachyon-staged Qwen bundle loads through Magnetar
- **WHEN** a model binding points at a `magnetar:` Tachyon-staged Qwen bundle
- **THEN** Tachyon constructs an authorized local bundle with
  `ModelArtifactSource::Tachyon`
- **AND** Magnetar's Hugging Face ingestor reads `config.json`, tokenizer
  metadata, generation metadata, and Safetensors payloads
- **AND** Tachyon receives a normalized Magnetar manifest and bounded payload
  source
- **AND** Tachyon does not parse the model payloads itself

#### Scenario: Magnetar trust policy is explicit
- **WHEN** Magnetar ingestion succeeds for a bundle whose manifest digest is not
  trusted by Tachyon's host-controlled model trust policy outside the artifact
  root
- **THEN** Tachyon rejects the binding before Provider execution
- **AND** parsing success, source kind, and local filesystem authorization do
  not grant trust
- **AND** trust metadata shipped inside the model artifact cannot self-authorize
  the bundle

#### Scenario: Reference CPU generation is allowed by CPU placement
- **WHEN** route policy selects CPU for an admitted Qwen bundle
- **THEN** Tachyon executes generation through Magnetar's Reference CPU
  production Qwen path
- **AND** mesh telemetry reports the real Magnetar Provider used for execution

#### Scenario: CUDA placement uses Magnetar device-resident decode
- **WHEN** route policy explicitly selects CUDA
- **THEN** Tachyon uses a real Magnetar `CudaProvider` when one is available
- **AND** Tachyon rejects the request when CUDA is unavailable instead of
  silently falling back to Reference CPU
- **AND** requests for multiple generated tokens on CUDA execute only through
  Magnetar's device-resident decode path

#### Scenario: Dynamic loading is target-aware
- **WHEN** a dynamic `magnetar:` model alias is requested for a specific
  accelerator
- **THEN** Tachyon lazy-loads the binding for that requested accelerator
- **AND** a CPU-loaded model cannot satisfy a GPU request
- **AND** an already-loaded GPU model for another alias cannot make a dynamic
  GPU request fall back to CPU

#### Scenario: OpenAI-shaped local requests are mapped to Magnetar contracts
- **WHEN** a local OpenAI chat request supplies messages, supported sampling
  parameters, a seed, max token budget, stop text, or streaming intent
- **THEN** Tachyon maps chat turns to `PromptInput::ChatMessages`,
  generation controls to `GenerationParameters`, stop text to
  `StopConditions`, and streaming output to `GenerationStreamEvent::Token`
  deltas
- **AND** Tachyon does not render Qwen chat templates or fabricate streaming
  chunks locally
- **AND** downstream streaming cancellation propagates to Magnetar so generation
  does not continue to the full requested token budget after disconnect

### Requirement: Active WIT inference surface remains Magnetar-scoped
The public `wit/ai/inference.wit` contract SHALL expose only the local inference
request/response function needed by Tachyon's Magnetar-backed runtime.
Historical LoRA adapter injection, per-call layer-wise memory profiles,
handle-based layer execution, and local multi-device topology validation SHALL
NOT remain in the active inference WIT package.

#### Scenario: Historical local execution contracts are absent
- **WHEN** `wit/ai/inference.wit` is inspected
- **THEN** it does not expose `adapter-id`, `infer-with-options`,
  `memory-profile`, `layer-execution`, or `parallel-execution`
- **AND** model format parsing, layer execution, tensor handles, and Provider
  execution remain Magnetar-owned

### Requirement: Local legacy text-generation compatibility is not an active inference surface
Tachyon SHALL NOT expose Cargo feature aliases, runtime modules, CI gates, or
canonical requirements that select a local legacy text-generation path. Historical
Qwen bindings and compatibility directories SHALL be rejected unless they satisfy
the active Magnetar production bundle contract.

#### Scenario: Old local text-generation feature names are unavailable
- **WHEN** a developer attempts to enable an obsolete local text-generation
  compatibility feature
- **THEN** Cargo reports that the feature does not exist
- **AND** the build does not compile a hidden local text-generation fallback

#### Scenario: Non-Magnetar local model directories are rejected
- **WHEN** a local Hugging Face-style directory is configured without matching
  the Magnetar production Qwen bundle contract
- **THEN** Tachyon rejects the binding with a typed unsupported-model error
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
- **THEN** the host compiles successfully without local production inference dependencies
- **AND** the default release and container workflows remain unchanged

#### Scenario: AI inference build links Magnetar runtime
- **WHEN** a developer builds `core-host` with `--features ai-inference`
- **THEN** the Magnetar runtime adapter, Hugging Face loader, tokenizer support,
  and selected Provider dependencies are compiled

#### Scenario: AI guest runs without ai-inference feature
- **WHEN** `core-host` is built without `--features ai-inference`
- **AND** an AI guest or route requires a model binding
- **THEN** execution fails gracefully with an error naming the missing
  `ai-inference` feature
