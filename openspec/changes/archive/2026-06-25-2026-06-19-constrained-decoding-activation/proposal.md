# Proposal: Activate Native Constrained Decoding (Close Spec/Code Drift)

## Why
Correction to the earlier audit: the constrained-decoding requirement *is* present in a merged spec — `openspec/specs/core-host/spec.md` already contains *"core-host MUST support native constrained decoding behind ai-inference"*, carried over from the archived change `2026-05-16-ai-constrained-decoding`. That archived change's own `tasks.md` has all 4 tasks unchecked, and a full repository scan confirms there is no trace of the feature in code: zero matches for `sample-constrained`, `sample_constrained`, `FsmLogitProcessor`, or the `llm-samplers`/`lru` dependencies anywhere in `Cargo.toml`/`*.rs`/`*.wit`.

In other words: **the spec describes a shipped capability that was never built.** The change was archived (moving its delta into the merged spec) without the implementation landing — a process gap, not a missing spec. Re-proposing the same requirements would duplicate spec text that already exists; what's missing is (a) the implementation and (b) a guard against this happening again silently.

## What Changes
1. Implement exactly what `core-host/spec.md`'s "core-host MUST support native constrained decoding behind ai-inference" requirement already describes: the `sample-constrained` WIT function, the FSM compiler with SHA-256-keyed LRU cache, and the logit-masking `FsmLogitProcessor`, all gated behind the `ai-inference` feature.
2. Add a CI check that fails the build if the `ai-inference` feature is enabled but the constrained-decoding module is absent, so a future archive-without-implementation cannot silently regress this capability again.
3. Update `wit/ai/inference.wit` (the real file, not just the spec prose) with the `sample-constrained` function.

## Non-Goals
- Does not change the FSM/grammar design from the original `2026-05-16-ai-constrained-decoding` proposal — that design is reused as-is.
- Does not add new constraint formats beyond JSON Schema in this pass.

## Impact
- **Affected capability**: `core-host` (existing requirement, implementation only — see "MODIFIED" delta below for the new CI-verification sub-requirement).
- **Affected code**: `wit/ai/inference.wit`, `core-host/Cargo.toml`, new `core-host/src/ai_inference/samplers.rs`, CI workflow.
- **Risk**: low — this closes a gap rather than opening new design space; the architecture was already reviewed and merged once.
