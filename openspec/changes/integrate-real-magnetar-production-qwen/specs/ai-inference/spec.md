## ADDED Requirements

### Requirement: Tachyon MUST load Qwen bundles through Magnetar production ingestion
For local Qwen model bindings, the host SHALL delegate production model parsing and normalization to Magnetar's public production ingestion API. Tachyon SHALL construct a `ProductionModelSource::authorized_local_bundle` using `ModelArtifactSource::Tachyon` for Tachyon-staged bundles, invoke the Hugging Face ingestor, load the real tokenizer through the ingestor's tokenizer implementation, evaluate trust through `ModelTrustStore`, and build a Magnetar production Qwen fixture before generation.

#### Scenario: Tachyon-staged Qwen bundle loads through Magnetar
- **WHEN** a model binding points at a Tachyon-staged Qwen bundle containing `config.json`, `tokenizer.json`, tokenizer configuration, generation configuration, and Safetensors payloads
- **THEN** Tachyon hands the authorized bundle root to Magnetar production ingestion
- **AND** Magnetar returns a normalized model manifest and bounded payload source
- **AND** Tachyon does not parse Safetensors bytes, tokenizer JSON, or Qwen architecture tensors itself

#### Scenario: Untrusted ingested bundle fails before materialization
- **WHEN** Magnetar production ingestion succeeds for a bundle whose manifest is not trusted by Tachyon's model trust policy
- **THEN** loading fails before Provider resource materialization
- **AND** Tachyon reports a trust-shaped local inference error

### Requirement: Tachyon MUST execute production Qwen through real Magnetar Providers
The host SHALL execute admitted Qwen generation through Magnetar's real production Qwen path: real tokenizer, `ModelInstance`, Qwen Component, prepared execution plan, memory manager, and a real Provider. Reference CPU execution SHALL use Magnetar's Reference CPU path. CUDA execution SHALL use a real `CudaProvider`.

#### Scenario: CPU policy generates through Reference CPU
- **WHEN** route policy selects CPU for an admitted Qwen bundle
- **THEN** Tachyon invokes Magnetar production Qwen generation on the Reference CPU Provider
- **AND** the response is generated from the real tokenizer and model instance

#### Scenario: CUDA policy uses a real CudaProvider
- **WHEN** route policy explicitly selects CUDA for an admitted Qwen bundle
- **THEN** Tachyon constructs and passes a real Magnetar `CudaProvider`
- **AND** a successful response cannot have silently used Reference CPU as a fallback

### Requirement: CUDA multi-token generation MUST fail closed until device-resident decode exists
Until Magnetar provides device-resident multi-step CUDA decode, Tachyon SHALL NOT claim full CUDA generation support. CUDA placement MAY run the supported prefill / first-token path, but a request that explicitly requires CUDA and more than one generated token SHALL fail with an unsupported-capability error instead of falling back to CPU or copying KV history through host memory.

#### Scenario: Explicit CUDA multi-token request is rejected
- **WHEN** a request explicitly selects CUDA and asks for more than one generated token
- **THEN** Tachyon rejects the request with an unsupported-capability error
- **AND** Tachyon does not silently execute the request on Reference CPU
- **AND** Tachyon does not implement a GPU-to-host-to-GPU KV-cache workaround

#### Scenario: CPU route may generate multiple tokens
- **WHEN** route policy selects Reference CPU for the same Qwen bundle
- **THEN** Tachyon may execute multi-token generation through Magnetar's CPU path
- **AND** the response is not reported as CUDA execution

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
