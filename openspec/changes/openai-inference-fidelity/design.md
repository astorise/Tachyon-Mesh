## Context

The `real-candle-llm-runtime` change made `guest-openai` run real Candle
inference, but explicitly deferred chat templates and richer sampling as
non-goals. The accelerator ABI is unary (`compute(model-id, prompt) -> string`)
and the mesh `handler` export returns a fully buffered `response` (`body:
list<u8>`), so neither sampling fidelity nor streaming existed yet. This change
closes that gap.

## Goals / Non-Goals

**Goals:**
- Configurable, OpenAI-compatible sampling (temperature, top_p, seed) that stays
  deterministic by default.
- Render the model's own chat template for structured `messages`.
- Honour `stop` sequences with correct trimming, including while streaming.
- Provide true incremental streaming: a host generation engine that emits
  fragments, an accelerator streaming primitive, and an OpenAI SSE surface.

**Non-Goals:**
- Tool/function calling, logprobs, `n > 1`, or a `usage` token-count block (the
  accelerator returns text only).
- Streaming for the GPU/NPU/TPU accelerator interfaces (CPU-first, where the
  real backend runs).
- Multi-architecture support beyond the existing Llama family.

## Decisions

### Sampling lives in the host, via candle's `LogitsProcessor`

Token selection is a host concern (it owns the logits). `temperature`/`top_p`/
`seed` are parsed from the generation JSON and resolved into a `Sampling`
policy: absent/`<= 0` temperature is `ArgMax` (greedy, seed-independent),
otherwise multinomial with optional nucleus filtering when `top_p` is in
`(0, 1)`. This keeps every existing determinism test green and makes un-seeded
sampling reproducible via a fixed default seed.

### Chat templating lives in the host, not the guest

Only the host has the model files. The guest sends structured `messages`; the
host renders them with the checkpoint's `chat_template` (from
`tokenizer_config.json`) using `minijinja` + `minijinja-contrib` pycompat (real
HF templates call `.strip()`/`.split()`). A checkpoint without a template falls
back to a generic `role: content` rendering that ends on an open assistant turn.
The generation JSON accepts either `messages` (preferred) or a raw `prompt`
(back-compat), so the accelerator WIT is unchanged for the buffered path.

### Streaming is a pull-based resource, not an ABI rewrite

The mesh `handler` response is buffered, so streaming is layered rather than
replacing the unary contract:

1. **Engine.** `generate_streaming` runs the exact same decode as `generate`
   but emits each newly decoded, stop-trimmed fragment through a callback. The
   concatenation of fragments equals the buffered output. A hold-back of the
   trailing bytes within one stop-length prevents a partial stop match from
   leaking over the wire; emission is codepoint-boundary safe.
2. **Accelerator primitive.** `tachyon:accelerator/cpu` gains
   `compute-stream(model-id, prompt) -> token-stream` and a `token-stream`
   resource with `next() -> option<string>`. The host runs the decode on a
   dedicated thread that pushes fragments into a channel the resource drains.
   Streaming bypasses the batch scheduler (a single sequence; the backend
   serialises its own execution). The sealed-alias scope gate is unchanged.
3. **HTTP transport.** A generic `tachyon:mesh/response-body` interface gives the
   guest a `streaming-response` resource (`begin(status, headers)` then `write`).
   The host routes a request carrying `Accept: text/event-stream` to a streaming
   guest-execution path that runs the guest on a dedicated thread with the sink
   pre-installed, awaits the `begin` headers over a oneshot, and returns an axum
   `Body::from_stream` fed by a bounded channel — so headers and first bytes
   reach the client before generation ends. A guest that never calls `begin`
   falls back to buffered delivery through the same channel pair. Because the
   OpenAI surface must remain a user FaaS, the SSE framing lives in `guest-openai`
   (it pulls `compute-stream` fragments and emits `chat.completion.chunk` frames
   terminated by `data: [DONE]`); the host transport is interface-agnostic. This
   mirrors the existing websocket execution path.

### Risks / Trade-offs

- The host-side engine, the accelerator streaming primitive, sampling, chat
  templates, and stop are unit-testable natively. The end-to-end HTTP wire path
  (streaming guest-execution + guest SSE export) is covered by an integration
  test that drives `execute_streaming_guest` and asserts the concatenated SSE
  deltas equal the buffered content; it requires the `wasm32-wasip2`
  `guest-openai` artifact. The `quality` CI job builds that artifact and runs the
  test with `--features ai-inference` and `TACHYON_REQUIRE_GUEST_OPENAI=1`, so a
  missing artifact fails the job instead of silently skipping (a local run
  without the artifact still skips gracefully).
- Streaming bypasses QoS batching. Acceptable: a streamed request is a single
  sequence and decode is bounded by `max_new_tokens`.
- Streaming requests are dispatched with no `RouteResponseGuard`, so they do not
  yet participate in graceful-drain response accounting; long-lived SSE streams
  could outlive a drain. A follow-up can thread a guard through the transport.
