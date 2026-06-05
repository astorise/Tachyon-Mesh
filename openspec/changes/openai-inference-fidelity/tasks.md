## 1. Sampling

- [x] 1.1 Resolve `temperature`/`top_p`/`seed` into a sampling policy; `<= 0` or
      absent temperature is deterministic greedy.
- [x] 1.2 Drive token selection with candle's `LogitsProcessor`.
- [x] 1.3 Default to a fixed seed when a sampled request omits `seed`.
- [x] 1.4 Preserve all existing determinism tests (greedy unchanged).
- [x] 1.5 Add tests: seeded reproducibility, greedy collapse, policy resolution.

## 2. Chat templates

- [x] 2.1 Load the model's `chat_template`/special tokens from
      `tokenizer_config.json` once at model load.
- [x] 2.2 Render structured `messages` with minijinja + pycompat; fall back to a
      generic rendering when the checkpoint has no template.
- [x] 2.3 Accept either `messages` or a raw `prompt` in the generation request.
- [x] 2.4 Add tests: real template (`.strip()` via pycompat), generic fallback.

## 3. Stop sequences

- [x] 3.1 Bound and sanitise the caller's stop list.
- [x] 3.2 Halt generation on a matched stop and trim the match (and after) from
      the output.
- [x] 3.3 Hold back the trailing bytes within one stop-length while streaming so
      a partial match is never emitted.
- [x] 3.4 Add tests: earliest-match selection, end-to-end truncation.

## 4. Streaming engine and accelerator primitive

- [x] 4.1 Add `generate_streaming` emitting fragments that concatenate to the
      buffered output; make `generate` a thin accumulator over it.
- [x] 4.2 Expose `AiInferenceRuntime::stream_component_prompt` and a
      `BackendModel::stream_text` (buffered emit-once fallback for non-LLM
      backends).
- [x] 4.3 Add `compute-stream` + a `token-stream` resource to
      `tachyon:accelerator/cpu`; implement the host resource over a thread +
      channel; keep the sealed-alias scope gate.
- [x] 4.4 Add tests: streamed fragments reconstruct the buffered output.

## 5. Guest passthrough

- [x] 5.1 Forward `messages` structurally and `top_p`/`seed`/`stop` (string or
      array) to the host; omit unset params so host defaults apply.
- [x] 5.2 Add tests for the generation request encoding.

## 6. HTTP streaming transport (validated where the wasm guest runs)

- [x] 6.1 Add a generic streaming guest-execution path that flushes response
      body chunks as the guest produces them (mirroring the websocket path).
- [x] 6.2 Add the host import the guest writes body chunks to, and a streaming
      axum body fed by it.
- [x] 6.3 `guest-openai`: on `stream: true`, pull from `compute-stream` and emit
      `chat.completion.chunk` SSE frames terminated by `data: [DONE]` with
      `content-type: text/event-stream`.
- [x] 6.4 Integration test: an OpenAI streaming client receives incremental
      frames whose deltas concatenate to the non-streamed response.

## 7. Documentation

- [x] 7.1 Document the generation request fields (`messages`, `top_p`, `seed`,
      `stop`) and sampling/template/stop semantics in the runtime.
- [x] 7.2 Document the streaming transport and its time-to-first-token behaviour
      once the wire path lands.
