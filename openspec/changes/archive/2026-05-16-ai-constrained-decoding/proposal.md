# Proposal: Native Constrained Decoding (FSM)

## Why
Advanced AI agents and FaaS workflows generating synthetic datasets or performing tool-calls require LLMs to output strictly formatted data (e.g., valid JSON matching a specific schema). Validating this output post-generation leads to high failure rates and expensive retry loops. Evaluating regular expressions token-by-token within the isolated WebAssembly module introduces unacceptable CPU latency and context-switching overhead.

1. **Inefficiency:** Wasm-based token validation creates a severe bottleneck on the critical inference loop.
2. **Hallucination:** Unconstrained LLMs frequently break schema syntax (missing quotes, trailing commas), breaking downstream JSON parsers.
3. **Core Bloat:** Integrating complex grammar parsing and FSM libraries by default would bloat the `core-host` binary for non-AI workloads.

## What Changes
Introduce a native `LogitProcessor` within the `core-host` inference engine (using Candle), driven by a Finite State Machine (FSM).
1. **WIT Extension:** Add a `sample-constrained` capability to the `tachyon:mesh@1.1.0` contract, accepting an optional JSON Schema string.
2. **FSM Compilation:** The core-host compiles the schema into an FSM graph.
3. **Logit Masking:** Before token sampling, the logit processor checks the FSM's current state. Any logit corresponding to a token that violates the schema grammar is masked to $-\infty$, mathematically forcing the model to select a valid syntax token.

## Impact
- **FSM Compilation Cost:** Building the FSM from a schema is computationally expensive.
  - *Mitigation:* Implement an LRU cache mapping the SHA-256 hash of the JSON Schema to its compiled FSM representation.
- **Tokenizer Coupling:** The FSM must map grammar rules to the specific model's vocabulary IDs.
  - *Mitigation:* Ensure the `LogitProcessor` receives a reference to the active `Tokenizer` during initialization.
- **Zero-Overhead Policy:** - *Mitigation:* Gate the FSM and sampling dependencies behind the `ai-inference` feature flag.
