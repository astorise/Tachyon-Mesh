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
  trusted by Tachyon's model trust policy
- **THEN** Tachyon rejects the binding before Provider execution
- **AND** parsing success, source kind, and local filesystem authorization do
  not grant trust

#### Scenario: Reference CPU generation is allowed by CPU placement
- **WHEN** route policy selects CPU for an admitted Qwen bundle
- **THEN** Tachyon executes generation through Magnetar's Reference CPU
  production Qwen path
- **AND** mesh telemetry reports the real Magnetar Provider used for execution

#### Scenario: CUDA placement is fail-closed until device-resident decode exists
- **WHEN** route policy explicitly selects CUDA
- **THEN** Tachyon uses a real Magnetar `CudaProvider` when one is available
- **AND** Tachyon rejects the request when CUDA is unavailable instead of
  silently falling back to Reference CPU
- **AND** requests for more than one generated token on CUDA fail closed until
  Magnetar provides device-resident multi-step decode

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

### Requirement: Host optionally exposes WASI-NN imports without a local text-generation backend
The `core-host` runtime SHALL define an `ai-inference` Cargo feature that links
the `wasi_ephemeral_nn` preview1 host functions for legacy WASI guests without
changing the default host build. This compatibility surface is limited to
guest-facing WASI-NN imports and SHALL NOT provide a local text-generation
fallback.

#### Scenario: Default host builds without AI inference
- **WHEN** a developer builds `core-host` without enabling `ai-inference`
- **THEN** the host compiles successfully without the optional AI imports
- **AND** the default release and container workflows remain unchanged

#### Scenario: AI inference build links WASI-NN imports
- **WHEN** a developer builds `core-host` with `--features ai-inference`
- **THEN** the legacy preview1 linker registers the `wasi_ephemeral_nn` imports
- **AND** legacy guests can resolve the `wasi-nn` host functions at
  instantiation time

#### Scenario: AI guest runs without ai-inference feature
- **WHEN** `core-host` is built without `--features ai-inference`
- **AND** an AI guest or route requires a model binding
- **THEN** execution fails gracefully with an error naming the missing
  `ai-inference` feature
