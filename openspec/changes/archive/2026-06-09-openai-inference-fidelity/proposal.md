## Why

`guest-openai` now runs real `/v1/chat/completions` inference through the Candle
LLM runtime, but at low fidelity: it flattened chat turns into a fixed
`role: content` prompt (ignoring the model's own chat template), decoded greedily
while discarding `temperature`/`seed`, ignored `top_p` and `stop`, and could only
return a single buffered response (no streaming). OpenAI clients therefore got
correct-but-blunt output and no token streaming.

This change raises the inference path to OpenAI fidelity: real sampling, the
model's own chat template, stop sequences, and a streaming transport primitive
for true time-to-first-token.

## What Changes

- **Sampling.** The Candle runtime selects tokens with a per-request
  `LogitsProcessor` honouring `temperature`, `top_p` (nucleus), and `seed`.
  `temperature <= 0` (or absent) collapses to deterministic greedy decoding, so
  the existing reproducible-by-default contract is preserved. An un-seeded
  sampled request uses a fixed default seed so it stays reproducible.
- **Chat templates.** A structured `messages` request is rendered through the
  model's own `chat_template` (loaded from `tokenizer_config.json`) with a Jinja
  engine plus the Python-compatibility method set, so real instruct templates
  work. Checkpoints without a template fall back to a generic rendering.
- **Stop sequences.** Generation halts once any `stop` string appears in the
  decoded text, and the match (and anything after it) is trimmed — including a
  hold-back so a stop split across tokens is never partially emitted while
  streaming.
- **Streaming.** The runtime decodes incrementally (`generate_streaming`), and
  the `tachyon:accelerator/cpu` WIT gains a `compute-stream`/`token-stream`
  resource so a caller can pull decoded fragments as they are produced. The
  `guest-openai` `/v1/chat/completions` surface gains OpenAI-compatible
  `stream: true` Server-Sent Events, flushed incrementally over a streaming
  guest-execution path.
- **Guest passthrough.** `guest-openai` forwards `messages` structurally and the
  OpenAI sampling knobs (`top_p`, `seed`, `stop`) to the host.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `ai-inference`: configurable sampling, model chat-template rendering, stop
  sequences, and incremental streaming generation plus the accelerator
  streaming primitive.
- `openai-compatible-faas`: `/v1/chat/completions` runs real inference (no longer
  a stub), forwards sampling parameters, and supports streaming responses.
- `core-host`: a generic incremental body-flush streaming transport
  (`tachyon:mesh/response-body`) with a buffered fallback, plus the scope-gated
  `kv-partition`/`graph` linker fix that lets the user-role FaaS linker
  instantiate components importing those interfaces.

## Impact

- Affected code: `core-host/src/ai_inference.rs`,
  `core-host/src/ai_inference/candle_llm_runtime.rs`,
  `core-host/src/host_core/component_hosts.rs`,
  `core-host/src/host_core/guest_runtime.rs`, `core-host/src/host_core/app_runtime.rs`,
  `core-host/src/network/mod.rs`, `wit/accelerator/accelerator-cpu.wit`,
  `wit/tachyon.wit` (the `response-body` interface), `examples/guest-openai/`, and
  the AI inference + streaming integration tests.
- Affected dependencies: `minijinja` + `minijinja-contrib` (pycompat) under the
  existing `ai-inference` feature; no dependency is added to default builds.
- The WIT changes are additive (`compute-stream` on the accelerator, the
  `response-body` interface imported by `faas-guest`), so existing guests are
  unaffected. Streaming is CPU-first, where the real Candle backend runs, and the
  streaming HTTP path is gated behind `ai-inference`.
