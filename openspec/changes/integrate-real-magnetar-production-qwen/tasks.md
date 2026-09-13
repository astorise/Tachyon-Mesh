## 1. Dependencies and Build Gates

- [x] 1.1 Add pinned Magnetar dependencies for `magnetar-runtime` and the generic inference Component adapter at `fabe36f34df5ae4f3c5ccd94e4b13eb1d5695ae8`.
- [x] 1.2 Replace Candle compatibility feature aliases used for active inference with Magnetar-oriented feature gates while preserving non-AI default builds.
- [x] 1.3 Verify `cargo check -p core-host --features ai-inference` resolves the pinned Magnetar graph.

## 2. Real Magnetar Adapter

- [x] 2.1 Replace the local Magnetar facade types with a thin adapter over Magnetar inference Component source, trust, placement, residency, and invocation calls.
- [x] 2.2 Map Tachyon `magnetar:` model bindings to `ModelArtifactSource::Tachyon` authorized local bundles without parsing model artifacts in Tachyon.
- [x] 2.3 Keep tokenizer and chat-template loading behind Magnetar's inference Component adapter.
- [x] 2.4 Forward CPU/CUDA as generic placement constraints instead of constructing concrete Providers in Tachyon core.

## 3. Fail-Closed Policy and Capabilities

- [x] 3.1 Reject explicit CUDA placement when Magnetar CUDA Provider is unavailable instead of silently falling back to CPU.
- [x] 3.2 Allow explicit CUDA multi-token generation only through Magnetar's real device-resident decode path.
- [x] 3.3 Remove Tachyon-owned fake Magnetar capability and memory-residency constructs from active routing metadata.
- [x] 3.4 Derive mesh-visible AI capabilities from real Magnetar Provider metadata/status.

## 4. Tests

- [x] 4.1 Add CPU E2E coverage for Tachyon-shaped Component invocation through Magnetar, trust evaluation, residency, and CPU generation.
- [x] 4.2 Add fail-closed tests for untrusted bundles, self-trusting bundles, unavailable CUDA, and unsupported request fields.
- [x] 4.3 Add GPU-gated CUDA multi-token coverage through Magnetar's CUDA execution path.
- [x] 4.4 Add a zero-test / hardware-required guard so GPU CI cannot pass when no critical CUDA assertion ran.

## 5. CI and Specification Cleanup

- [x] 5.1 Replace active `candle-cuda` GPU proof commands with Magnetar production-loading CPU/GPU checks.
- [x] 5.2 Remove Candle-oriented canonical spec requirements that no longer describe the active local inference path.
- [x] 5.3 Run formatting, clippy, focused tests, and feature compile checks required by the change.
- [x] 5.4 Remove historical local runtime compatibility aliases, modules, GPU workflow, and active documentation now that Magnetar adoption is complete.

## 6. Magnetar Post-Closure Integration Update

- [x] 6.1 Pin Magnetar to `fabe36f34df5ae4f3c5ccd94e4b13eb1d5695ae8` after production loading, streaming, device-resident CUDA multi-token decode, and the generic inference Component adapter landed upstream.
- [x] 6.2 Replace buffered/facade generation calls with `ProductionGenerationRequest` and provider-specific Magnetar generation entry points.
- [x] 6.3 Map OpenAI chat messages to `PromptInput::ChatMessages` and map supported generation parameters and stop sequences into Magnetar contracts.
- [x] 6.4 Use Magnetar streaming events and `GenerationStreamEvent::Token.text_delta` for Tachyon streaming instead of buffering full generation first.
- [x] 6.5 Remove Tachyon's obsolete CUDA multi-token rejection and validate CUDA multi-token generation through Magnetar.
- [x] 6.6 Move Tachyon model trust policy outside artifact bundles and reject self-trust attempts from model-local files.
- [x] 6.7 Remove stale constrained-decoding WIT/spec/CI guards that referenced deleted local `FsmLogitProcessor` and `sample-constrained` surfaces.

## 7. Post-Audit Corrective Scope

- [x] 7.1 Make dynamic Magnetar model loading target-aware so CPU/GPU requests cannot be satisfied by a model lazy-loaded for a different accelerator.
- [x] 7.2 Enforce requested-vs-loaded accelerator equality for CPU, GPU, NPU, and TPU placements before generation.
- [x] 7.3 Remove historical LoRA, layer-wise, per-call memory-profile, and multi-device execution contracts from the active local inference WIT/spec surface.
- [x] 7.4 Make `system-faas-model-broker` format-neutral: no GGUF/Safetensors detection, no model `format` sidecar declaration, and no auth-session LoRA prewarm instruction.
- [x] 7.5 Add Magnetar streaming cancellation coverage proving downstream stop propagates through Tachyon instead of completing the full generation.
- [x] 7.6 Strengthen the real CUDA multi-token test to assert the generated token count, not only a non-empty response.

## 8. PR #412 Production Closure

- [x] 8.1 Preserve the full `guest-openai` host request envelope for local Magnetar models instead of rejecting Tachyon-owned control fields.
- [x] 8.2 Superseded by task 9.6: keep tool-call dialect semantics out of Tachyon core and fail closed unless the responsible guest/Component boundary handles them.
- [x] 8.3 Wire `max_generation_ms` to real deadline enforcement for local Magnetar execution.
- [x] 8.4 Either support structured-output controls on the Magnetar path or reject unsupported schema requests as invalid requests instead of runtime/server failures.
- [x] 8.5 Replace per-request Magnetar Runtime/Provider/model materialization with a persistent loaded ModelInstance lifecycle.
- [x] 8.6 Add sequential and concurrent tests proving one resident model is reused by multiple generation sessions.
- [x] 8.7 Track real model/checkpoint acceptance in Magnetar/Component lanes separately from fast Tachyon Component-boundary CI.
- [x] 8.8 Remove provider-name string matching from accelerator classification.
- [x] 8.9 Update PR/README documentation to the actual pinned Magnetar SHA and supported CUDA multi-token state.

## 9. Component Boundary Closure

- [x] 9.1 Replace the model-specific Tachyon adapter boundary with a generic Magnetar inference Component adapter.
- [x] 9.2 Remove model-family, model-format, tokenizer, chat-template, model-instance, and model-architecture knowledge from `core-host/src/ai_inference/magnetar_runtime.rs`.
- [x] 9.3 Remove direct `core-host` dependencies on concrete Magnetar loader and Provider crates; concrete loaders/providers are now behind Magnetar's Component adapter.
- [x] 9.4 Move the checked-in inference Component registration behind Magnetar's adapter instead of embedding a model-specific Component artifact in Tachyon core.
- [x] 9.5 Keep Tachyon's active local runtime state as a loaded inference Component handle, provider advertisement, placement, and routing metadata.
- [x] 9.6 Move model execution controls such as tool-call dialect handling and structured-output guarantees out of Tachyon core; unsupported local controls now fail closed as invalid Component invocations.
- [x] 9.7 Rewrite the canonical AI inference, memory delegation, and GitHub Actions specs so Tachyon owns Component transport, trust, routing, QoS, admission, generic placement constraints, and telemetry only.
- [x] 9.8 Add Magnetar adapter tests covering payload mapping, deadline propagation, and fail-closed tool/structured-output controls outside Tachyon core.
- [x] 9.9 Add compile validation proving `core-host --features magnetar-cuda` resolves through the generic Component adapter.

## 10. True External Component Closure

- [x] 10.1 Make the inference Component an explicit runtime artifact selected outside Magnetar's model implementation.
- [x] 10.2 Resolve the selected `*.component.wasm` and sidecar manifest from the Tachyon-staged artifact root and pass the bytes to Magnetar.
- [x] 10.3 Remove the implicit/default Qwen Component from the generic `LoadedInferenceComponent` loading path.
- [x] 10.4 Rename Tachyon's trust boundary to Component/artifact trust and keep host-controlled trust outside artifact roots.
- [x] 10.5 Remove tool-call dialect interpretation from `core-host`; the registry only transports opaque Component metadata.
- [x] 10.6 Replace the active Tachyon inference WIT payload with an opaque Component invocation contract.
- [x] 10.7 Remove model-execution controls from active Tachyon core configuration and keep only generic placement constraints.
- [x] 10.8 Add tests proving explicit Component artifacts are required and legacy local adapter bindings are rejected before execution.
- [x] 10.9 Add a CI architecture guard preventing model-family, tokenizer, model-format, model-execution, and compiled-in default Component knowledge from re-entering production core paths.
