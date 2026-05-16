# Technical Specification: Constrained Decoding Architecture

## Purpose
Constrained decoding lets the host enforce structured LLM output during token sampling instead of relying on retry loops after invalid JSON or tool-call payloads have already been generated.

## ADDED Requirements

### Requirement: Constrained decoding dependencies MUST be feature gated
The `core-host` crate SHALL add the necessary FSM/sampling crates conditionally under the `ai-inference` feature.

```toml
[dependencies]
# ... existing dependencies ...
llm-samplers = { version = "0.0.7", optional = true }
lru = { version = "0.12", optional = true }

[features]
# Expand the existing feature
ai-inference = ["dep:candle-core", "dep:llm-samplers", "dep:lru"]
```

#### Scenario: Default build excludes constrained decoding crates
- **WHEN** `core-host` is built without `--features ai-inference`
- **THEN** `llm-samplers` and `lru` are not linked into the host binary

### Requirement: Inference WIT MUST expose constrained sampling
The `wit/ai/inference.wit` contract SHALL extend the existing `layer-execution` interface with constrained sampling.

```wit
package tachyon:mesh@1.1.0;

interface layer-execution {
    // ... existing types and functions (load-layer, forward-layer) ...

    /// Samples the next token from the final logits tensor, constrained by an optional JSON schema
    sample-constrained: func(
        logits: tensor-handle,
        json-schema: option<string>
    ) -> result<u32, string>;
}
```

#### Scenario: Guest requests schema-constrained token sampling
- **WHEN** a guest calls `sample-constrained` with a logits tensor handle and an optional JSON Schema string
- **THEN** the host returns a selected token id or a human-readable error

### Requirement: core-host MUST cache compiled FSMs by schema hash
The constrained decoding implementation SHALL compile JSON Schema strings into FSM state and cache the compiled representation in a thread-safe LRU cache keyed by SHA-256 schema hash.

#### Scenario: Repeated schema avoids recompilation
- **WHEN** the same JSON Schema is used for multiple constrained samples
- **THEN** core-host reuses the cached FSM instead of recompiling it

### Requirement: Logit masking MUST advance FSM state after sampling
The logit processor SHALL mask disallowed logits to negative infinity before sampling and advance the FSM state after a token is selected.

```rust
#[cfg(feature = "ai-inference")]
pub mod constrained {
    use candle_core::{Tensor, Result};
    use lru::LruCache;
    use std::num::NonZeroUsize;
    use std::sync::Mutex;

    static FSM_CACHE: OnceLock<Mutex<LruCache<String, Arc<CompiledFSM>>>> = OnceLock::new();

    pub struct FsmLogitProcessor {
        fsm_state: Arc<CompiledFSM>,
        current_node: usize,
        tokenizer: Arc<Tokenizer>,
    }

    impl FsmLogitProcessor {
        pub fn process(&mut self, logits: &mut Tensor) -> Result<()> {
            // 1. Ask the FSM which string prefixes are valid from the current node
            let allowed_tokens = self.fsm_state.get_allowed_tokens(self.current_node, &self.tokenizer);

            // 2. Apply -INF mask to all logits NOT in allowed_tokens
            // This relies on native Candle tensor operations for speed
            mask_disallowed_logits(logits, allowed_tokens)?;

            Ok(())
        }

        pub fn advance_state(&mut self, selected_token: u32) {
            self.current_node = self.fsm_state.transition(self.current_node, selected_token);
        }
    }
}
```

#### Scenario: Disallowed tokens cannot be selected
- **WHEN** the FSM only allows a subset of token ids for the current state
- **THEN** logits outside that allowed set are masked to negative infinity
- **AND** the sampled token advances the FSM state for the next generation step
