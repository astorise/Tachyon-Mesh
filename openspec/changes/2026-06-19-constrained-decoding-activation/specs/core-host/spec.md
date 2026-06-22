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
- **THEN** it verifies that `sample_constrained`/`sample-constrained` and `FsmLogitProcessor` symbols exist in `core-host/src`
- **AND** the build fails if either symbol is absent, preventing a recurrence of a merged spec requirement with no matching implementation
