# Implementation Tasks

- [ ] **Task 1: WIT contract (actually implement, not just spec)**
  - Add `sample-constrained` to `wit/ai/inference.wit`'s `layer-execution` interface as already documented in `openspec/specs/core-host/spec.md`.
  - Regenerate host bindings.

- [ ] **Task 2: Conditional dependencies**
  - Add `llm-samplers` and `lru` to `core-host/Cargo.toml` as optional dependencies linked to the `ai-inference` feature.

- [ ] **Task 3: FSM compilation and caching**
  - Create `core-host/src/ai_inference/samplers.rs` (guarded by `#[cfg(feature = "ai-inference")]`).
  - Implement the thread-safe SHA-256-keyed LRU cache for compiled FSM graphs.

- [ ] **Task 4: Logit masking integration**
  - Implement `FsmLogitProcessor`: mask disallowed logits to `-inf` based on FSM state, sample, advance FSM state.
  - Wire it into the existing generation loop (buffered and streaming paths from `ai-inference`'s "runtime streams decoded fragments incrementally" requirement) so constrained sampling works for both.

- [ ] **Task 5: CI drift guard**
  - Add the build+grep verification step from `design.md` so a future archive of a spec delta without matching code fails CI.

- [ ] **Task 6: Tests**
  - Unit test: a JSON-Schema-constrained generation never produces a token violating the schema's FSM.
  - Unit test: repeated requests with the same schema hit the LRU cache (no recompilation).
  - Regression test: default build (`ai-inference` off) does not link `llm-samplers`/`lru`.
