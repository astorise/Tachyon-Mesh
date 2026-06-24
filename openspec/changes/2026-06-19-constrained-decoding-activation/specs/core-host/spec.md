# core-host Delta

## MODIFIED Requirements

### Requirement: core-host MUST support native constrained decoding behind ai-inference
The `core-host` crate SHALL keep constrained decoding dependencies optional under the `ai-inference` feature, extend `wit/ai/inference.wit` with `sample-constrained`, and provide a native logit processor that compiles JSON Schema strings into cached FSM state before masking invalid token logits. CI SHALL verify that this requirement is implemented in code whenever it is asserted in the spec, so the requirement cannot be merged as spec text without a corresponding implementation.

#### Scenario: Guest samples logits with an optional JSON Schema
- **WHEN** a guest calls `sample-constrained` with a logits tensor handle and a JSON Schema
- **THEN** core-host samples only tokens allowed by the compiled schema FSM
- **AND** repeated calls with the same schema reuse the cached FSM by schema hash

#### Scenario: Core host is built without constrained decoding dependencies
- **WHEN** `core-host` is built without `--features ai-inference`
- **THEN** `llm-samplers`, `lru`, and the constrained decoding sampler module are not linked into the binary

#### Scenario: CI fails if the requirement is specified but not implemented
- **WHEN** the CI workflow builds `core-host --features ai-inference`
- **THEN** it verifies that `sample-constrained` and `FsmLogitProcessor` symbols exist in the codebase
- **AND** the build fails if either symbol is absent, preventing a recurrence of a merged spec requirement with no matching implementation

## Implementation status as of this change

This requirement is now implemented, with two deviations from the literal scenario
text above, both intentional and narrower in scope than "any JSON Schema":

- **No `llm-samplers` dependency.** It is a generic sampling-algorithm crate
  (top-k/top-p/etc.), not a JSON-Schema/grammar compiler — the project's existing
  `candle_transformers::generation::LogitsProcessor`/`Sampling` already covers
  token sampling. `lru` was already present (added by an earlier, unrelated
  change) and did not need a new `Cargo.toml` entry. The "not linked into the
  binary" scenario therefore still holds for the default build, just without
  `llm-samplers` ever existing as a dependency to begin with.
- **The CI drift guard checks two locations, not one.** `sample-constrained`
  only exists as a WIT function name in `wit/ai/inference.wit` — there is no
  Rust symbol literally spelled `sample_constrained` in `core-host/src`, so a
  grep for that string against `core-host/src` alone (as originally proposed
  in this change's own `design.md`) would never match. The actual CI step
  (`.github/workflows/ci.yml`, after "Check ai-inference host build") asserts
  `FsmLogitProcessor` exists in `core-host/src` and `sample-constrained` exists
  in `wit/ai/inference.wit` — the two places this capability actually lives.

The grammar/FSM compiler itself (`core-host/src/ai_inference/samplers.rs`)
supports flat top-level `object` schemas with scalar/string-enum properties, or
a top-level scalar schema — no nesting, arrays, `$ref`, or `oneOf`/`anyOf`,
rejected at compile time with a typed error rather than mishandled silently.
Per-step logit masking does a full vocabulary scan (decode-and-check per
candidate id), documented as a known performance follow-up rather than
optimized in this pass. Verified by 7 unit tests in `samplers.rs` and 2
integration tests in `candle_llm_runtime.rs`, plus the full `ai_inference::`
suite (114 tests, 0 regressions) and a clean
`cargo clippy --features ai-inference --all-targets -- -D warnings -D
clippy::unwrap_used`.
