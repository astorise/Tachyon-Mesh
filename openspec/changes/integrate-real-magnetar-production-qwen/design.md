## Context

Before this corrective change, Tachyon accepted `magnetar:` model bindings through a local compatibility facade that rejected real Qwen execution instead of calling Magnetar. The previous cutover archive also left canonical specs and GPU CI steps describing `candle-cuda` as the active local inference path.

Magnetar now exposes the public embedder path for production Qwen loading at commit `b235783abb2b0c92843febfa3ff29f30745e1934`: `ProductionModelSource`, `HuggingFaceIngestor`, `ModelTrustStore`, `production_qwen_fixture`, `ProductionGenerationRequest`, provider generation, and streaming generation events. Tachyon should become a transport, provenance, routing, and QoS layer around that API, not a parser or execution-engine shim.

## Goals / Non-Goals

**Goals:**

- Pin Magnetar crates to `b235783abb2b0c92843febfa3ff29f30745e1934`.
- Replace the Tachyon-local Magnetar facade with a thin adapter over Magnetar's public production Qwen APIs.
- Treat Tachyon-staged model directories as `ModelArtifactSource::Tachyon` through `ProductionModelSource::authorized_local_bundle`.
- Use Magnetar's real Hugging Face ingestor, tokenizer, trust store, ModelInstance, Qwen Component, prepared execution plan, Reference CPU Provider, and CudaProvider.
- Fail closed when CUDA is explicitly requested but unavailable, and allow CUDA multi-token generation only through Magnetar's real device-resident decode path.
- Update specs and CI so Candle is no longer described as the active local inference runtime and GPU checks cannot pass vacuously.

**Non-Goals:**

- Do not implement Magnetar's device-resident multi-step CUDA decode in Tachyon.
- Do not add Safetensors, tokenizer, BF16/F16 conversion, Tensor Resource, Qwen graph, KV-cache, or Provider execution logic to Tachyon.
- Do not emulate CUDA generation through CPU fallback or GPU-host-GPU KV round trips.
- Do not silently fall back from explicitly requested CUDA to Reference CPU.

## Decisions

1. **Pin Magnetar through the vendored submodule and path dependencies.**
   - Use the exact Magnetar revision from the completed production Qwen loading audit so CI resolves the same API surface everywhere.
   - Tachyon depends on the vendored `vendor/Magnetar` crates instead of workstation-local `../Magnetar` paths or unpublished registry versions.

2. **Keep the Tachyon adapter narrow.**
   - The adapter maps a Tachyon model binding into `ProductionModelSource::authorized_local_bundle(ModelArtifactSource::Tachyon(..), root)`, invokes `HuggingFaceIngestor`, loads the real tokenizer, creates a `ModelTrustStore`, builds `production_qwen_fixture`, and calls Magnetar generation.
   - Tachyon records routing telemetry and enforces policy, but it never fabricates Magnetar kernel/tensor IDs or hardware capabilities.

3. **Make placement explicit and fail-closed.**
   - CPU placement constructs a real Reference CPU Provider and uses `run_production_qwen_generation_for_provider_with_request`.
   - CUDA placement constructs a real `CudaProvider` and uses `run_production_qwen_generation_for_provider_with_request`.
   - CUDA multi-token requests are no longer rejected by Tachyon; Magnetar owns the provider capability check and device-resident decode implementation.
   - Dynamic lazy loading receives the requested accelerator and materializes the binding for that target. A loaded binding whose accelerator differs from the request is rejected for every accelerator class, including CPU and GPU.

4. **Forward production request semantics.**
   - Tachyon maps OpenAI chat turns to `PromptInput::ChatMessages` and does not render Qwen chat templates locally.
   - Tachyon maps accepted generation parameters and stop sequences into Magnetar `GenerationParameters` and `StopConditions`.
   - Streaming uses `run_production_qwen_generation_for_provider_streaming` and emits `GenerationStreamEvent::Token.text_delta`.

5. **Replace Candle CI proof with Magnetar proof.**
   - CPU CI covers real production ingestion and generation through Magnetar.
   - GPU CI covers CUDA multi-token generation with a real CudaProvider and includes an assertion that the GPU-critical path actually ran.
   - Test selection commands must fail if they match zero tests.

6. **Correct, do not rewrite, the archived history.**
   - The old archive remains historical evidence. This change adds a corrective successor that records why the earlier completion was insufficient and what replaces it.

7. **Remove historical compatibility from active contracts.**
   - `wit/ai/inference.wit` keeps only the Magnetar-backed request/response surface.
   - Per-call LoRA adapter injection, layer-wise memory profiles, layer tensor handles, and local multi-device topology validation are not public Tachyon promises after the cutover.
   - `system-faas-model-broker` remains an artifact transport/provenance component. It verifies declared files and publishes Magnetar-owned upload events, but it does not detect GGUF/Safetensors, write a model `format` sidecar, or synthesize tenant LoRA prewarm instructions.

## Risks / Trade-offs

- [Risk] Magnetar APIs at the pinned SHA use Rust 2024 while `core-host` is Rust 2021. -> Mitigation: Cargo supports mixed-edition dependencies; keep Tachyon code idiomatic 2021 and isolate Magnetar calls in one module.
- [Risk] Pulling `magnetar-provider-cuda` into ordinary CPU CI could require CUDA runtime libraries. -> Mitigation: keep CUDA dependency optional and gate CUDA checks behind the existing GPU runner path.
- [Risk] Accepted OpenAI request options can drift from Magnetar's production request contract. -> Mitigation: Tachyon maps supported fields explicitly and rejects unsupported local Magnetar fields instead of silently ignoring them.
- [Risk] Trust policy could be smuggled inside an artifact bundle. -> Mitigation: Tachyon reads trust from a host-controlled path outside the artifact root and ignores artifact-local trust files.

## Migration Plan

1. Add pinned Magnetar dependencies and feature gates.
2. Replace the local facade implementation with the real Magnetar adapter while keeping Tachyon's public inference APIs stable.
3. Remove fake Magnetar capability/residency types from Tachyon's runtime surface and map telemetry to real Provider metadata.
4. Update tests for CPU production Qwen, explicit trust rejection, self-trust rejection, non-Qwen rejection, CUDA multi-token generation, and GPU coverage.
5. Add post-audit tests for target-aware dynamic loading, accelerator mismatch rejection, streaming cancellation, format-neutral broker behavior, and exact CUDA multi-token counts.
6. Replace `candle-cuda` CI proof steps with Magnetar CPU/GPU checks and zero-test guards.
7. Run formatting, clippy, focused tests, and compile-only feature checks.

## Open Questions

- Whether Tachyon route configuration already carries an explicit generation token limit that can be mapped directly to Magnetar's manifest generation defaults, or whether this change needs a minimal internal per-request cap for CUDA policy enforcement.
- Whether the final PR should include a small CI helper script for zero-test guards or inline shell in `.github/workflows/ci.yml`.
