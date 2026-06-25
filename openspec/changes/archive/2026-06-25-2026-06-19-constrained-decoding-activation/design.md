# Design: Constrained Decoding Activation

This reuses, verbatim, the architecture already approved in the archived change `2026-05-16-ai-constrained-decoding` (see `openspec/changes/archive/2026-05-16-ai-constrained-decoding/design.md`-equivalent content, now living in `openspec/specs/core-host/spec.md`). No new design decisions are introduced; this document only records the verification mechanism added to prevent another spec/code drift.

## 1. What already exists (spec only, to be implemented as-is)
- `wit/ai/inference.wit` `layer-execution` interface gains `sample-constrained: func(logits: tensor-handle, json-schema: option<string>) -> result<u32, string>`.
- `core-host/Cargo.toml`: `llm-samplers` and `lru` as optional dependencies under the `ai-inference` feature.
- `core-host/src/ai_inference/samplers.rs` (feature-gated): SHA-256-keyed `LruCache<String, Arc<CompiledFSM>>`, and `FsmLogitProcessor` masking disallowed logits to `-inf` before sampling, advancing FSM state after each token.

## 2. New: drift-detection CI check
Because this exact gap (spec merged, code absent) is what slipped through last time, add a lightweight CI assertion that runs whenever `core-host/spec.md` references a Cargo feature-gated symbol set (here: `sample-constrained`, `FsmLogitProcessor`) — verifying the symbol exists in the codebase when `--features ai-inference` is built. This is intentionally narrow (not a generic spec-to-code linter); it targets the specific requirement this change implements.

```yaml
# .github/workflows (excerpt)
- name: Verify constrained decoding is implemented, not just specified
  run: |
    cargo build -p core-host --features ai-inference 2>&1 | tee build.log
    grep -q "sample_constrained\|sample-constrained" core-host/src -r
    grep -q "FsmLogitProcessor" core-host/src -r
```

## 3. Compatibility
- `ai-inference` feature remains off by default; no impact on the default host build.
- No change to the WIT contract beyond what was already specified and merged.
