## 1. Dependencies and Build Gates

- [x] 1.1 Add pinned Magnetar dependencies for `magnetar-runtime`, `magnetar-loader-huggingface`, `magnetar-provider-cpu`, and optional `magnetar-provider-cuda` at `b235783abb2b0c92843febfa3ff29f30745e1934`.
- [x] 1.2 Replace Candle compatibility feature aliases used for active inference with Magnetar-oriented feature gates while preserving non-AI default builds.
- [x] 1.3 Verify `cargo check -p core-host --features ai-inference` resolves the pinned Magnetar graph.

## 2. Real Magnetar Adapter

- [x] 2.1 Replace the local Magnetar facade types with a thin adapter over `ProductionModelSource`, `HuggingFaceIngestor`, `ModelTrustStore`, `production_qwen_fixture`, and Magnetar generation calls.
- [x] 2.2 Map Tachyon `magnetar:` model bindings to `ModelArtifactSource::Tachyon` authorized local bundles without parsing model artifacts in Tachyon.
- [x] 2.3 Load the real Hugging Face tokenizer through `magnetar-loader-huggingface` and pass it to Magnetar's Qwen production fixture.
- [x] 2.4 Use Reference CPU generation when route placement selects CPU and real `CudaProvider` generation when placement explicitly selects CUDA.

## 3. Fail-Closed Policy and Capabilities

- [x] 3.1 Reject explicit CUDA placement when Magnetar CUDA Provider is unavailable instead of silently falling back to CPU.
- [x] 3.2 Allow explicit CUDA multi-token generation only through Magnetar's real device-resident decode path.
- [x] 3.3 Remove Tachyon-owned fake Magnetar capability and memory-residency constructs from active routing metadata.
- [x] 3.4 Derive mesh-visible AI capabilities from real Magnetar Provider metadata/status.

## 4. Tests

- [x] 4.1 Add CPU E2E coverage for Tachyon-shaped Qwen bundle ingestion through Magnetar, trust evaluation, tokenizer loading, fixture construction, and Reference CPU generation.
- [x] 4.2 Add fail-closed tests for untrusted bundles, self-trusting bundles, non-Qwen bundles, unavailable CUDA, and unsupported request fields.
- [x] 4.3 Add GPU-gated CUDA multi-token coverage through real `CudaProvider`.
- [x] 4.4 Add a zero-test / hardware-required guard so GPU CI cannot pass when no critical CUDA assertion ran.

## 5. CI and Specification Cleanup

- [x] 5.1 Replace active `candle-cuda` GPU proof commands with Magnetar production-loading CPU/GPU checks.
- [x] 5.2 Remove Candle-oriented canonical spec requirements that no longer describe the active local inference path.
- [x] 5.3 Run formatting, clippy, focused tests, and feature compile checks required by the change.
- [x] 5.4 Remove historical local runtime compatibility aliases, modules, GPU workflow, and active documentation now that Magnetar adoption is complete.

## 6. Magnetar Post-Closure Integration Update

- [x] 6.1 Pin Magnetar to `b235783abb2b0c92843febfa3ff29f30745e1934` after production loading, streaming, and device-resident CUDA multi-token decode landed upstream.
- [x] 6.2 Replace buffered/facade generation calls with `ProductionGenerationRequest` and provider-specific Magnetar generation entry points.
- [x] 6.3 Map OpenAI chat messages to `PromptInput::ChatMessages` and map supported generation parameters and stop sequences into Magnetar contracts.
- [x] 6.4 Use Magnetar streaming events and `GenerationStreamEvent::Token.text_delta` for Tachyon streaming instead of buffering full generation first.
- [x] 6.5 Remove Tachyon's obsolete CUDA multi-token rejection and validate CUDA multi-token generation through the real `CudaProvider`.
- [x] 6.6 Move Tachyon model trust policy outside artifact bundles and reject self-trust attempts from model-local files.
- [x] 6.7 Remove stale constrained-decoding WIT/spec/CI guards that referenced deleted local `FsmLogitProcessor` and `sample-constrained` surfaces.
