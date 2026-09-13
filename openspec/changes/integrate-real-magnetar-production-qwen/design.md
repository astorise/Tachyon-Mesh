## Context

The previous corrective change replaced Tachyon's Candle-era local inference
path with Magnetar production execution, but the first PR implementation still
placed too much model knowledge in `core-host`: it directly referenced a model
family, a Hugging Face loader, tokenizer files, concrete Providers, and a
model-specific loaded-model type.

The final boundary is now Component-centric:

```text
Tachyon core
  -> Component artifact/provenance/trust/routing/QoS/placement/deadline
  -> Magnetar inference Component adapter
  -> Magnetar model ingestion/tokenizer/model instance/Provider/generation
```

Tachyon's core must be able to carry a new inference Component without adding
model-family Rust code, tokenizer parsing, concrete Provider construction, or
model-specific prompt shaping to `core-host`.

## Goals / Non-Goals

**Goals:**

- Pin Magnetar through the vendored submodule and path dependencies.
- Replace local Candle compatibility with a Magnetar-backed inference Component
  path.
- Keep Tachyon responsible for Component provenance, host-controlled trust,
  routing, QoS, admission, placement constraints, deadline policy, and streaming
  transport.
- Keep model-family, model-format, tokenizer, chat-template, model instance,
  and Provider implementation knowledge behind Magnetar.
- Fail closed when explicit CUDA placement is unavailable and when local
  invocation controls require guarantees the responsible inference layer does
  not provide.
- Update specs and CI so Candle is no longer described as the active local
  inference runtime and GPU checks cannot pass vacuously.

**Non-Goals:**

- Do not implement model loaders, tokenizer parsing, Provider construction,
  graph construction, or KV-cache management in Tachyon.
- Do not emulate CUDA generation through CPU fallback or GPU-host-GPU KV round
  trips.
- Do not implement tool-call dialects or structured-output guarantees in
  Tachyon core.

## Decisions

1. **Pin Magnetar through the vendored submodule and path dependencies.**
   - Tachyon depends on the vendored `vendor/Magnetar` crates so CI resolves
     the same API surface everywhere.

2. **Introduce a Magnetar-owned generic inference Component adapter.**
   - `magnetar-inference-component` depends on concrete Magnetar loader and
     Provider crates and exposes generic Component-facing types to Tachyon.
   - `core-host` no longer depends directly on `magnetar-loader-*` or
     `magnetar-provider-*` crates.
   - The adapter receives Component bytes and a Component manifest from Tachyon;
     it does not register or select an internal default Component.

3. **Keep the Tachyon adapter narrow.**
   - The adapter maps a `magnetar:` binding into an explicit Component artifact,
     Component source/provenance, trust, placement, and invocation payloads.
   - Tachyon records routing telemetry and enforces policy, but it never
     parses tokenizer/model files, constructs model fixtures, fabricates
     Magnetar kernel/tensor IDs, or creates concrete Providers.

4. **Make placement explicit and fail-closed.**
   - CPU/CUDA are generic placement constraints forwarded to Magnetar.
   - A loaded binding whose accelerator differs from the request is rejected
     for every accelerator class, including CPU and GPU.
   - Explicit CUDA placement fails if Magnetar reports CUDA unavailable.

5. **Move protocol/model controls out of Tachyon core.**
   - Tool-call dialects and structured-output guarantees belong to
     `guest-openai`, the inference Component, or Magnetar.
   - The local Component path rejects unsupported tool/structured-output
     controls as invalid invocations rather than converting them into prompt
     instructions.
   - Historical adapter/LoRA bindings are not local inference controls; local
     routes that still declare them are rejected before runtime construction.

6. **Replace Candle CI proof with Magnetar Component proof.**
   - CPU CI covers authorized Component invocation through Magnetar.
   - GPU CI covers CUDA multi-token generation through Magnetar and includes an
     assertion that the GPU-critical path actually ran.
   - Test selection commands must fail if they match zero tests.
   - A production-code architecture guard blocks model-family names, tokenizer
     contracts, model-format detection, model-execution knobs, and compiled-in
     default Component registration from the Tachyon core boundary.

7. **Correct, do not rewrite, the archived history.**
   - The old archive remains historical evidence. This change adds a
     corrective successor that records why the earlier completion was
     insufficient and what replaces it.

## Risks / Trade-offs

- [Risk] Magnetar's generic Component boundary still wraps today's production
  implementation internally. -> Mitigation: the coupling is moved behind a
  Magnetar crate; Tachyon core only sees Component-level contracts.
- [Risk] Pulling CUDA support into ordinary CPU CI could require CUDA runtime
  libraries. -> Mitigation: CUDA remains feature-gated and GPU execution stays
  behind the existing GPU runner path.
- [Risk] Accepted OpenAI request options can drift from the responsible
  Component contract. -> Mitigation: unsupported local Component controls fail
  closed instead of being silently prompt-mutated by Tachyon core.
- [Risk] Trust policy could be smuggled inside an artifact bundle. ->
  Mitigation: Tachyon reads trust from a host-controlled path outside the
  artifact root and ignores artifact-local trust files.

## Migration Plan

1. Add pinned Magnetar dependencies and feature gates.
2. Move concrete production loader/provider/model lifecycle code behind a
   Magnetar inference Component adapter.
3. Replace Tachyon's local adapter with a narrow Component invocation adapter.
4. Remove fake Magnetar capability/residency types from Tachyon's runtime
   surface and map telemetry to Magnetar metadata.
5. Update tests for CPU Component invocation, explicit trust rejection,
   self-trust rejection, unavailable CUDA, target-aware dynamic loading,
   explicit Component artifact presence, legacy adapter rejection, streaming
   cancellation, and exact CUDA multi-token counts.
6. Replace `candle-cuda` CI proof steps with Magnetar Component CPU/GPU checks
   and zero-test plus Component-boundary guards.
7. Run formatting, focused tests, feature compile checks, OpenSpec validation,
   and CI.
