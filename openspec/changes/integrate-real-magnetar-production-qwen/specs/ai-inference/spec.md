## ADDED Requirements

### Requirement: Tachyon MUST load Qwen bundles through Magnetar production ingestion
For local Qwen model bindings, the host SHALL delegate production model parsing and normalization to Magnetar's public production ingestion API. Tachyon SHALL construct a `ProductionModelSource::authorized_local_bundle` using `ModelArtifactSource::Tachyon` for Tachyon-staged bundles, invoke the Hugging Face ingestor, load the real tokenizer through the ingestor's tokenizer implementation, evaluate trust through a host-controlled `ModelTrustStore` outside the artifact bundle, and build a Magnetar production Qwen fixture before generation.

#### Scenario: Tachyon-staged Qwen bundle loads through Magnetar
- **WHEN** a model binding points at a Tachyon-staged Qwen bundle containing `config.json`, `tokenizer.json`, tokenizer configuration, generation configuration, and Safetensors payloads
- **THEN** Tachyon hands the authorized bundle root to Magnetar production ingestion
- **AND** Magnetar returns a normalized model manifest and bounded payload source
- **AND** Tachyon does not parse Safetensors bytes, tokenizer JSON, or Qwen architecture tensors itself

#### Scenario: Untrusted ingested bundle fails before materialization
- **WHEN** Magnetar production ingestion succeeds for a bundle whose manifest is not trusted by Tachyon's model trust policy
- **THEN** loading fails before Provider resource materialization
- **AND** Tachyon reports a trust-shaped local inference error

#### Scenario: Artifact-local trust policy is ignored
- **WHEN** a Tachyon-staged bundle contains a `tachyon-model-trust.json` or `.tachyon-model-trust.json` file that trusts its own manifest digest
- **THEN** Tachyon still treats the bundle as untrusted unless the host-controlled trust store outside the artifact root trusts that digest
- **AND** model artifacts cannot self-authorize by shipping trust metadata inside the bundle

### Requirement: Tachyon MUST execute production Qwen through real Magnetar Providers
The host SHALL execute admitted Qwen generation through Magnetar's real production Qwen path: real tokenizer, `ModelInstance`, Qwen Component, prepared execution plan, memory manager, and a real Provider. Reference CPU execution SHALL use Magnetar's Reference CPU Provider. CUDA execution SHALL use a real `CudaProvider`. Tachyon SHALL pass caller input through `ProductionGenerationRequest`, mapping chat messages to `PromptInput::ChatMessages`, supported generation parameters to `GenerationParameters`, and text stop sequences to `StopConditions`.

#### Scenario: CPU policy generates through Reference CPU
- **WHEN** route policy selects CPU for an admitted Qwen bundle
- **THEN** Tachyon invokes Magnetar production Qwen generation on the Reference CPU Provider
- **AND** the response is generated from the real tokenizer and model instance

#### Scenario: CUDA policy uses a real CudaProvider
- **WHEN** route policy explicitly selects CUDA for an admitted Qwen bundle
- **THEN** Tachyon constructs and passes a real Magnetar `CudaProvider`
- **AND** a successful response cannot have silently used Reference CPU as a fallback

#### Scenario: OpenAI chat request uses Magnetar chat input
- **WHEN** a local OpenAI chat request contains `messages`
- **THEN** Tachyon passes those messages to Magnetar as `PromptInput::ChatMessages`
- **AND** Tachyon does not render the Qwen chat template locally

#### Scenario: Request parameters and stops reach Magnetar
- **WHEN** a local OpenAI request supplies supported sampling parameters, seed, max token budget, or stop text
- **THEN** Tachyon maps them to Magnetar `GenerationParameters`, `ProductionGenerationRequest.max_new_tokens`, and `StopConditions`
- **AND** unsupported local Magnetar request fields fail closed instead of being silently ignored

### Requirement: CUDA multi-token generation MUST use Magnetar device-resident decode
Now that Magnetar provides device-resident multi-step CUDA decode, Tachyon SHALL NOT reject explicit CUDA requests solely because they ask for more than one generated token. Tachyon SHALL still fail closed when the real `CudaProvider` is unavailable and SHALL NOT emulate CUDA generation by falling back to CPU or copying KV history through host memory.

#### Scenario: Explicit CUDA multi-token request runs on CUDA
- **WHEN** a request explicitly selects CUDA and asks for at least sixteen generated tokens
- **THEN** Tachyon executes generation through Magnetar's real `CudaProvider`
- **AND** Tachyon does not silently execute the request on Reference CPU
- **AND** Tachyon does not implement a GPU-to-host-to-GPU KV-cache workaround

#### Scenario: CPU route may generate multiple tokens
- **WHEN** route policy selects Reference CPU for the same Qwen bundle
- **THEN** Tachyon may execute multi-token generation through Magnetar's CPU path
- **AND** the response is not reported as CUDA execution

### Requirement: Tachyon streaming MUST consume Magnetar generation events
For local Magnetar Qwen streaming, Tachyon SHALL use Magnetar's production streaming entry point and consume `GenerationStreamEvent::Token.text_delta`. Tachyon SHALL NOT buffer a full generation and then emit it as a fabricated streaming chunk.

#### Scenario: Streaming emits token deltas from Magnetar
- **WHEN** a local OpenAI request uses `stream: true`
- **THEN** Tachyon obtains token deltas from Magnetar streaming events
- **AND** cancellation from the downstream stream propagates with `ControlFlow::Break`

### Requirement: Tachyon MUST NOT fabricate Magnetar execution or hardware identities
The host SHALL consume Magnetar runtime, Provider, and Device contracts rather than creating local stand-ins for Magnetar handles or capability advertisements. Tachyon SHALL NOT define permanent local `PreparedKernelId`, `TensorId`, hardcoded `CUDA_0`, hardcoded VRAM/dtype support, environment-variable hardware truth, or pseudo Magnetar memory residency as the source of mesh routing truth.

#### Scenario: Capability advertisement comes from Magnetar Provider metadata
- **WHEN** Tachyon advertises local AI execution capacity to the mesh
- **THEN** the advertised Provider and Device data is derived from real Magnetar Provider metadata
- **AND** it is not derived from hardcoded Tachyon constants or `TACHYON_MAGNETAR_CUDA`

#### Scenario: Tachyon does not expose fake Magnetar handles
- **WHEN** the host executes Qwen through Magnetar
- **THEN** Tachyon does not mint local kernel or tensor handles to represent Magnetar-owned execution resources
- **AND** Tensor Resource identity and Provider execution remain inside Magnetar

## REMOVED Requirements

### Requirement: Candle LLM prefill is chunked and configurable
**Reason**: Production local Qwen execution is now delegated to Magnetar's Qwen Component and generation path rather than Tachyon's Candle LLM runtime.
**Migration**: Use Magnetar production Qwen ingestion and generation policy. Future prefill chunking requirements belong in the Magnetar contract or a Tachyon policy layer that does not inspect model tensors.

### Requirement: Downstream Candle quantization kernels MUST be consumed through the pinned fork
**Reason**: Candle is no longer the active local inference dependency for the Magnetar cutover path.
**Migration**: Quantization support must be represented through Magnetar model/provider capabilities when Magnetar exposes production quantized Qwen support.

### Requirement: Candle engine hot-swaps adapter weights and bounds context-switching overhead
**Reason**: Adapter execution through a Candle engine is not part of the real Magnetar production Qwen cutover.
**Migration**: LoRA and adapter multiplexing must be re-specified against Magnetar Components and Providers before being re-enabled for local production inference.

### Requirement: `candle-cuda` MUST be documented as the single CUDA switch
**Reason**: CUDA coverage for the active local inference path must use Magnetar Provider/CUDA gates, not Candle feature switches.
**Migration**: Use Magnetar CUDA Provider CI checks and explicit fail-closed placement requirements.
