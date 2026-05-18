# Implementation Tasks

- [ ] **Task 1: WIT Expansion**
  - Update `wit/ai/inference.wit` to include the `sample-constrained` function accepting an optional `json-schema` string.
  - Re-generate host bindings.

- [ ] **Task 2: Conditional Dependencies**
  - Add `llm-samplers` and `lru` to `Cargo.toml` as optional dependencies linked to the `ai-inference` feature.

- [ ] **Task 3: FSM Compilation and Caching**
  - Create `core-host/src/ai_inference/samplers.rs` (guarded by `#[cfg(feature = "ai-inference")]`).
  - Implement a thread-safe LRU cache to store compiled FSM graphs based on the schema string hash to prevent CPU spikes on recurring tool-calls.

- [ ] **Task 4: Logit Masking Integration**
  - Implement the custom logit processor that takes the logits tensor handle from the Wasm module, applies the $-\infty$ mask based on the FSM state, and samples the final token.
  - Ensure the internal FSM state advances properly after the token is selected, maintaining consistency across the FaaS generation loop.
