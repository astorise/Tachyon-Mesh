## Context

Tachyon currently accepts `magnetar:` model bindings, but `core-host/src/ai_inference/magnetar_runtime.rs` is a local compatibility facade that rejects real Qwen execution with `not implemented yet`. The previous cutover archive also left canonical specs and GPU CI steps describing `candle-cuda` as the active local inference path.

Magnetar now exposes the public embedder path for production Qwen loading at commit `0104bcbda4383b572d278faf5a24cb93ab3fd072`: `ProductionModelSource`, `HuggingFaceIngestor`, `ModelTrustStore`, `production_qwen_fixture`, and `run_production_qwen_generation(_for_provider)`. Tachyon should become a transport, provenance, routing, and QoS layer around that API, not a parser or execution-engine shim.

## Goals / Non-Goals

**Goals:**

- Pin Magnetar crates to `0104bcbda4383b572d278faf5a24cb93ab3fd072`.
- Replace the Tachyon-local Magnetar facade with a thin adapter over Magnetar's public production Qwen APIs.
- Treat Tachyon-staged model directories as `ModelArtifactSource::Tachyon` through `ProductionModelSource::authorized_local_bundle`.
- Use Magnetar's real Hugging Face ingestor, tokenizer, trust store, ModelInstance, Qwen Component, prepared execution plan, Reference CPU Provider, and CudaProvider.
- Fail closed when CUDA is explicitly requested but unavailable, or when CUDA is requested for unsupported multi-token decode.
- Update specs and CI so Candle is no longer described as the active local inference runtime and GPU checks cannot pass vacuously.

**Non-Goals:**

- Do not implement Magnetar's device-resident multi-step CUDA decode in Tachyon.
- Do not add Safetensors, tokenizer, BF16/F16 conversion, Tensor Resource, Qwen graph, KV-cache, or Provider execution logic to Tachyon.
- Do not claim full CUDA generation until Magnetar supports device-resident multi-token decode.
- Do not silently fall back from explicitly requested CUDA to Reference CPU.

## Decisions

1. **Pin Magnetar with git dependencies, not a local path.**
   - Use the exact Magnetar revision from the completed production Qwen loading audit so CI resolves the same API surface everywhere.
   - Local `../Magnetar` paths are useful for inspection but would make CI and PR review depend on developer workstation layout.

2. **Keep the Tachyon adapter narrow.**
   - The adapter maps a Tachyon model binding into `ProductionModelSource::authorized_local_bundle(ModelArtifactSource::Tachyon(..), root)`, invokes `HuggingFaceIngestor`, loads the real tokenizer, creates a `ModelTrustStore`, builds `production_qwen_fixture`, and calls Magnetar generation.
   - Tachyon records routing telemetry and enforces policy, but it never fabricates Magnetar kernel/tensor IDs or hardware capabilities.

3. **Make placement explicit and fail-closed.**
   - CPU placement uses `run_production_qwen_generation`.
   - CUDA placement constructs a real `CudaProvider` and uses `run_production_qwen_generation_for_provider`.
   - CUDA requests with `max_tokens > 1` return an unsupported-capability error until Magnetar's device-resident multi-step decode is available. CPU may generate multiple tokens when policy selected CPU explicitly.

4. **Replace Candle CI proof with Magnetar proof.**
   - CPU CI covers real production ingestion and generation through Magnetar.
   - GPU CI covers CUDA prefill / first-token generation with a real CudaProvider and includes an assertion that the GPU-critical path actually ran.
   - Test selection commands must fail if they match zero tests.

5. **Correct, do not rewrite, the archived history.**
   - The old archive remains historical evidence. This change adds a corrective successor that records why the earlier completion was insufficient and what replaces it.

## Risks / Trade-offs

- [Risk] Magnetar APIs at the pinned SHA use Rust 2024 while `core-host` is Rust 2021. -> Mitigation: Cargo supports mixed-edition dependencies; keep Tachyon code idiomatic 2021 and isolate Magnetar calls in one module.
- [Risk] Pulling `magnetar-provider-cuda` into ordinary CPU CI could require CUDA runtime libraries. -> Mitigation: keep CUDA dependency optional and gate CUDA checks behind the existing GPU runner path.
- [Risk] Magnetar generation API currently returns full generation output rather than Tachyon's existing streaming callback semantics. -> Mitigation: preserve buffered generation first; streaming may emit the final generated chunk until Magnetar exposes a token streaming embedder API.
- [Risk] Existing route configs that requested `magnetar:` CUDA multi-token generation will fail where they previously reached only a facade. -> Mitigation: this is intentional fail-closed behavior; operators can choose CPU placement or `max_tokens = 1` for CUDA prefill validation.

## Migration Plan

1. Add pinned Magnetar dependencies and feature gates.
2. Replace the local facade implementation with the real Magnetar adapter while keeping Tachyon's public inference APIs stable.
3. Remove fake Magnetar capability/residency types from Tachyon's runtime surface and map telemetry to real Provider metadata.
4. Update tests for CPU production Qwen, explicit trust rejection, non-Qwen rejection, CUDA fail-closed multi-token policy, and GPU prefill coverage.
5. Replace `candle-cuda` CI proof steps with Magnetar CPU/GPU checks and zero-test guards.
6. Run formatting, clippy, focused tests, and compile-only feature checks.

## Open Questions

- Whether Tachyon route configuration already carries an explicit generation token limit that can be mapped directly to Magnetar's manifest generation defaults, or whether this change needs a minimal internal per-request cap for CUDA policy enforcement.
- Whether the final PR should include a small CI helper script for zero-test guards or inline shell in `.github/workflows/ci.yml`.
