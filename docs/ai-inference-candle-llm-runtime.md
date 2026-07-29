# Candle LLM Runtime

Tachyon's `core-host --features ai-inference` build supports a real Candle-backed
LLM path for local text-generation model bindings. This path is separate from
legacy Candle ONNX/WASI-NN graph loading and from the ModelOpt/NVFP4 loader.

## Supported Fixture Family

The first supported family is `TachyonTinyCausalLM`, identified by:

```json
{
  "model_type": "tachyon_tiny_causal_lm",
  "architectures": ["TachyonTinyCausalLM"]
}
```

The model directory must contain:

- `config.json`
- `tokenizer.json`
- `model.safetensors`

`model.safetensors` must contain an F32 tensor named `next_token_logits` with
shape `[vocab_size]`. The runtime loads the tokenizer with `tokenizers`, loads
the tensor through Candle safetensors support, and performs deterministic greedy
selection with Candle `argmax`.

This fixture is intentionally small so CI can validate real Candle execution
without downloading external artifacts.

## Prompt Input

The first `U8` inference tensor is interpreted as either plain UTF-8 prompt text
or a JSON generation request.

Plain prompt:

```text
hello
```

JSON request:

```json
{
  "prompt": "hello",
  "max_new_tokens": 1,
  "temperature": 0.0,
  "seed": 7
}
```

Default generation is deterministic. Requests are bounded by prompt byte count,
prompt token count, batch size, and the host max-new-token cap.

## Prompt Limits

The prompt token and byte budgets are derived from the checkpoint's context
window rather than being flat constants. They used to be 4096 tokens and 16 KiB
— chosen when the runtime only served short prompts — so an agentic client
sending a source file plus its tool definitions was rejected outright on a model
whose context window had plenty of room.

`prompt_limits_for(max_position_embeddings)` computes both:

- **Tokens**: the context window minus reserved generation headroom, floored at
  `MIN_PROMPT_TOKENS` (itself never allowed to exceed the real window). The
  reservation matters: without it a prompt could pass validation and still leave
  the decode loop with zero positions left, which returns empty output rather
  than an error. It is *proportional* — a quarter of the window, capped at
  `HOST_MAX_NEW_TOKENS` — because that constant is the largest generation the
  host will allow, not what every request needs; subtracting it outright would
  leave a 4k-context checkpoint with almost no prompt budget.
- **Bytes**: `tokens × PROMPT_BYTES_PER_TOKEN`, clamped between `MIN_PROMPT_BYTES`
  (16 KiB — the flat cap this replaced, kept as a floor so the change can only
  ever *widen* what a checkpoint accepts) and `MAX_PROMPT_BYTES_CEILING` (4 MiB,
  so a million-token context cannot justify a byte budget that is its own
  denial-of-service).

The byte cap is a pre-tokenization guard on what one request can pull into host
memory; the token cap is the semantic limit. That is why the byte side is
deliberately generous — a tokenizer with long tokens fits far more text into a
token than the per-token estimate assumes, and under-budgeting rejects valid
prompts.

A 32k-context checkpoint therefore accepts ~28.7k prompt tokens and ~115 KB,
against 4096 tokens and 16 KiB before.

The reservation is a *default*, not this request's budget, so the pair is
checked too. A caller that names `max_new_tokens` it cannot fit beside its
prompt gets a request error naming what would fit — the alternative is a decode
loop that reaches the context boundary partway through and returns a truncated
answer with no signal. A caller that names no budget has no expectation to
violate, so the default is clamped to the window instead of refused.

## Incremental Detokenization

Every decode step needs the text generated so far — to test stop sequences, and
to emit the streaming delta. Decoding the whole token sequence on each step is
the obvious way to get it and is what the loops used to do, but it is O(n²) in
the generated length: at 256 tokens that is invisible, at several thousand it is
millions of redundant per-token decodes.

A naive fix does not work. A BPE/SentencePiece decode is not the concatenation
of its tokens' decodes: a multi-byte character can span several tokens (the
prefix decodes to a replacement character that a later token retroactively
replaces), and decoding a sub-sequence can gain or lose a leading space.

The decoder that solves this is
`candle_transformers::generation::IncrementalDecoder`, not a local one. It keeps
a bounded trailing *window* of tokens, re-decodes only that window, and appends
the difference against the previous window decode — so any sub-sequence artifact
is present in both decodes and cancels. Windowing is only enabled for tokenizers
whose decoder has a *bounded context*: the suffix check proves the shortened
decode matches now, not that a future token cannot rewrite text before the
anchor, which a decoder applying a regex replacement can do. Unbounded decoders
degrade to whole-sequence decoding rather than corrupting output.

This runtime shipped that implementation first and then upstreamed it, so what
remains here is the seam: `IncrementalDecoder::from_tokenizer(&self.tokenizer)`
in each of the three generation loops, `push(token)` per step, and `text()` —
which is always equal to a whole-sequence decode — fed to the stop-sequence scan
and the delta emitter. Emission accounting stays local because the loops hold
back a `hold` suffix of their own, sized by the longest stop sequence.

`from_tokenizer` rather than `new` because `new` clones the tokenizer, and the
continuous-batching loop needs one decoder per row. Holding the tokenizer by
reference costs nothing, and the batch loop builds a single decoder and clones
it per row, so neither the vocabulary copy nor the decoder-context resolution
is paid `batch` times.

The invariant — that the incremental text equals what decoding the whole
sequence would produce — is asserted step by step in
`incremental_detokenization_matches_the_model_tokenizer` against the real
fixture tokenizer. The decoder's own edge cases (characters split across
tokens, unbounded decoder detection, re-anchoring) are covered upstream in
candle; what is tested here is that this runtime's tokenizer and its
`decode_generated` stay interchangeable with the incremental path.

Stop-sequence scanning is deliberately left as a full-text search per step: it
is also O(n²), but it is a `memchr`-backed substring search over bytes, orders of
magnitude cheaper per byte than tokenizer work, and keeping it whole-text avoids
any question about missing an earlier match.

## Mock and Unsupported Bindings

Mock output is only available through explicit mock bindings such as:

```text
mock:llama3
```

Non-mock model bindings that are not supported Candle LLM directories, ONNX
guest-loaded graphs, or ModelOpt/NVFP4 directories fail during initialization
with an actionable error. They do not fall back to `MOCK_LLM_RESPONSE`.

## NVFP4 Boundary

ModelOpt/NVFP4 remains a separate loader and kernel capability. A detected
directory is never interpreted as Llama merely because it contains
safetensors. The registered Qwen 3.5 MoE compatibility profile executes through
its dedicated hybrid runtime; unmatched architectures return an explicit
unsupported-architecture error.

## Single-Device GPU Execution

The single-device (`GpuDistribution::Single`) path executes on a real CUDA
device only for a Llama-family checkpoint, and only on a build with the
`candle-cuda` feature: `load_safetensors` resolves
`Device::cuda_if_available(0)` when the binding requests a non-`cpu` device
for a Llama checkpoint, falling back to `Device::Cpu` silently if no
physical GPU is found at runtime (the same convention the tensor/pipeline/
expert-parallel engines already use). Every other architecture on the
single-device path — Qwen2/3, Gemma2/3, Phi3/4, the DeepSeek family — and
every build without `candle-cuda` still return the existing typed
`UnsupportedModel` error for a non-`cpu` request; multi-GPU placement
already had its own real CUDA path via `distribution_mode:
tensor_parallelism`/`pipeline_parallelism`/`expert_parallelism`, unaffected
by this.

## Generation Deadline

`max_new_tokens` is a poor proxy for how long a request occupies a scheduler
slot: the same 4096 tokens are ~100 s on a GPU and a quarter of an hour on a
CPU, and throughput varies as much between two models on one device as it does
between devices. Sizing the token cap per hardware class would still be wrong by
an order of magnitude in both directions.

So every generation carries a wall-clock deadline. `DEFAULT_GENERATION_DEADLINE`
is 300 s — matching the upstream backend's request timeout, so local and remote
bindings look the same to a caller — overridable per request with
`max_generation_ms` and clamped to `HOST_MAX_GENERATION_DEADLINE` (1 hour).

The deadline is anchored at parse time, not at the first decode step, so
tokenizing and prefilling a long prompt count against it: that work holds the
same slot. When it expires the loop stops the way an exhausted token budget
does — flushing what was generated — rather than erroring: a partial answer is
worth more than a failure, and freeing the slot is the point.

A batch is per row: each row is retired at its own deadline while rows with time
left keep decoding, rather than the whole batch stopping at the earliest — which
truncated every co-batched answer to the shortest budget in the group. The one
part rows genuinely cannot decide separately is the shared prefill forward pass,
so that is abandoned only when *every* row has expired; it is checked between
prefill chunks, as the single-sequence and prefix-cache paths are.

`HOST_MAX_NEW_TOKENS` stays at 4096 as the coarse safety valve against an
unbounded loop; it is no longer the de-facto regulator of slot occupancy.

## Token Usage

`usage` is reported from the decode loop, never estimated. `prompt_tokens` is
the tokenizer's own encoding of the prompt; `completion_tokens` is what the loop
actually appended. Re-tokenizing the output text would have been far easier and
would have been wrong — a decode of *n* tokens does not re-encode to *n* tokens,
so the number would have looked plausible and been unfalsifiable.

Getting the count out of the loop is the whole design problem. A decode loop has
roughly a dozen exits — stop sequence matched, EOS, deadline elapsed, budget
exhausted, context window full — spread across three loops and fifteen dispatch
arms, several of them behind `candle-cuda`. Returning the count would have made
correctness depend on remembering every one. Instead the loops write through a
`TokenSink`, which carries the emit callback *and* the counters, so the count is
right at every exit by construction. `record_token` sits beside the one
`generated.push` in each loop.

The count is not the number of `emit` calls: a token can produce no text (its
bytes complete a character only once a later token arrives), and text is held
back while a stop sequence might still match.

### Where usage reaches the client

| Path | Reported | Why |
|---|---|---|
| non-streaming | always | no extra frame to break a client with, so nothing to gate on |
| `stream: true` + `stream_options.include_usage` | yes | dedicated channel beside the fragment stream |
| `stream: true` alone | no | matches OpenAI, whose extra trailing chunk breaks clients that index `choices[0]` |
| upstream (`openai:`) binding, buffered | always | every OpenAI-shaped upstream returns `usage` on this route |
| upstream (`openai:`) binding, streamed | when volunteered | read opportunistically, never requested |
| mock | word counts | no tokenizer; deterministic and obviously synthetic |
| vendor | no | text over a pipe, nothing to count |

The two paths reach the client differently. Streaming counts travel *beside* the
token stream, not through it: they are only known once decoding ends, by which
point the last fragment has already gone down the channel, and sending them as a
final fragment would have made them indistinguishable from model output.
`token-stream.usage()` returns `none` until then.

The buffered path has no trailing frame, so the counts come back with the text:
`compute-detailed` returns a `generation` record of both. It is a separate
function rather than a change to `compute`'s return type, so every caller that
only wants the text is untouched.

Underneath, both share one seam. `InferenceOutput` — bytes plus
`Option<TokenUsage>` — is what `BackendModel::execute` returns and what the
scheduler's response channel carries, so a buffered request keeps its batching
and still reports what it cost. `Option` rather than a zero default is the whole
point: it makes "this backend cannot measure" a distinct answer from "this
generation cost nothing", which the client would otherwise believe.

An absent `usage` always means "not measured". Zeros are never published as if
they were measured — a zero `usage` is a claim that the generation cost nothing,
and for context-window accounting that is worse than saying nothing.

For upstream bindings the counts are read opportunistically rather than
requested. OpenAI only emits `usage` on a stream when
`stream_options.include_usage` is set, but sending that field to an upstream
that does not recognize it risks a 400 that breaks streaming outright — a bad
trade for a reporting field.

## GGUF Families

Architecture dispatch is candle's, not this crate's:
`candle_transformers::models::quantized_lm` owns the
`general.architecture` → backend registry, the per-family metadata namespace,
and the `(architecture, device, dtype)` admission check. `load_gguf` calls
`Architecture::from_content`, `quantized_lm::context_length` and
`Architecture::check_device_support`, so an unsupported family fails with the
registry's own message and the supported set cannot drift from what candle can
actually build. Every family in `SUPPORTED_ARCHITECTURES` is reachable —
`llama`, `gemma`/`gemma2`/`gemma3`/`gemma-embedding`, `glm4`, `lfm2`, `phi2`,
`phi3`, `qwen2`, `qwen3`, `qwen3moe`.

Construction goes through `quantized_lm::from_gguf_with` as well, so no
per-family dispatch remains in this crate at all — `LoadedModel::Gguf` holds a
`Mutex<Box<dyn QuantizedLm>>`. An earlier revision could not do this: the trait
had no `Send` bound, so the boxed form could not live in the shared registry's
`Mutex` even though every concrete backend behind it is `Send`, and this crate
carried an enum over the concrete types purely to keep the auto trait. `Send`
is now on the trait and the enum is gone.

The architecture is still resolved once up front, before handing the file to
`from_gguf_with`, for one reason: an unrecognized family is a property of the
checkpoint and has to surface as `UnsupportedModel`, not the `InvalidComponent`
that every other load failure maps to.

Metadata keys are architecture-prefixed, so the context-length lookup reads
`{architecture}.context_length` through `quantized_lm::arch_metadata` —
hardcoding `llama.` would have silently fallen back to the default window for
every other family.

Two gates decide whether a checkpoint loads, and they must agree.
`ModelArchitecture::from_gguf_architecture` runs first and refuses anything it
does not recognize; `ArchitectureCapabilities.gguf` then decides whether that
architecture accepts the format. A family present in candle's loader but absent
from the first gate is dead code — which is exactly what happened to `glm4`,
`lfm2`, `phi2` and `qwen3moe`, advertised in this document and refused before
the loader ran. `every_loadable_gguf_family_passes_the_architecture_gate` now
walks `quantized_lm::SUPPORTED_ARCHITECTURES` and fails if the two drift again.

`gguf` stays `false` for families candle has no quantized module for (Phi4,
DeepSeek), so an unsupported checkpoint fails at validation rather than at
load. `qwen3moe` is `true` despite its fused expert
path being CUDA-only with an F16/BF16 working dtype: `check_device_support`
refuses the CPU and Metal bindings at load, whereas refusing the format outright
would also refuse the CUDA binding, which works.

### Quantized GGUF on CUDA

GGUF follows the same rule. `load_gguf` resolves the binding's device rather
than pinning `Device::Cpu`, so a Llama GGUF bound to a non-`cpu` device on a
`candle-cuda` build uploads its quantized weights to the GPU and decodes there;
`LoadedModel::Gguf` carries that device so prefill and decode build their input
tensors on it. This is what makes a 4-bit checkpoint usable on a consumer GPU:
the safetensors path loads F32 (or BF16 under `paged_attention`), which costs
4 (or 2) bytes per parameter, while a Q4_K_M GGUF costs roughly half a byte.

Only block types the target device can actually multiply are accepted. The rule
is `Device::supports_qmatmul(dtype)` in candle, not a table copied into this
crate: `Q8_1` and `Q8K` are activation formats rather than weight formats —
CUDA has no matmul kernel for them at all, and Metal handles them only on the
mat-vec path — so a checkpoint carrying them is rejected at load — with the offending tensor and block type named — instead of
failing at the first decode step with VRAM already claimed. Everything else
passes, including `F32`/`F16`/`BF16`, which `QMatMul::from_arc` dequantizes into
a dense tensor rather than dispatching to a quantized kernel. The CPU backend
implements `vec_dot` generically for every dtype, so a `cpu` binding is never
gated.

### Quantized GGUF on Metal

The `candle-metal` feature wires Apple GPU execution for GGUF bindings
(`device: metal`). It is deliberately narrower than `candle-cuda`: it activates
candle's Metal backend and nothing else, because the CUDA-only extras that
feature pulls in — flash-attn, paged attention, cudarc, NVML — have no Metal
counterpart here. Only `load_gguf` resolves a Metal device; safetensors families
still build against `Device::Cpu`, so enabling the feature does not silently
move them.

One difference from the CUDA path is worth knowing: `metal` does not fall back
to the host. An operator asking for a device the machine does not have has a
configuration error, and running on CPU instead would hide it behind a
mysterious slowdown. (The CUDA path keeps its fallback because that convention
predates this loader and the parallel engines rely on it.)

The block-type scan *does* apply to Metal. Metal's quantized backend parses
every GGML block type and can dequantize all of them, and it even has
`kernel_mul_mv_q8_1_f32` / `kernel_mul_mv_q8_K_f32` for the mat-vec path. What
it does not have is the mat-mat equivalent used for prefill, or `get_rows` for
embedding lookup — both return `UnsupportedDTypeForOp`. A checkpoint stored in
those dtypes would therefore decode at batch 1 and fail at prefill, which is why
`supports_qmatmul` gates them on both accelerators alike.

Metal is tracked as a separate gate from CUDA rather than folded into one "GPU"
flag, so a Metal binding cannot accept a CUDA-only optimization.

`paged_attention`, `cuda_graph_decode`, and `flashinfer_attention` remain
safetensors-only and are rejected for a GGUF binding: `QuantizedLlama` owns its
KV cache and decode path internally, never sees `hardware_strategy`, and exposes
no block-table seam. Accepting any of them for GGUF would silently run ordinary
quantized attention while reporting the optimization as enabled.

One subtlety: the block-type check runs *after* the device is resolved, not
before, and against whichever device was resolved.
`Device::cuda_if_available` falls back to `Device::Cpu` when no physical GPU is
present, and a checkpoint that lands on that fallback executes through the host
kernels, which support every block type — scanning first would reject a
perfectly runnable model on a `candle-cuda` build with no GPU attached.

## OpenAI-Compatible Upstream Bindings

A model binding whose `path` uses the `openai:` scheme is served by an external
OpenAI-compatible server rather than by a local Candle runtime. The mesh keeps
routing, QoS, authorisation, and the `/ai/v1` surface; only the tensor math runs
out of process. This is the supported way to serve a model whose architecture or
quantization Tachyon has no verified loader for — an AWQ checkpoint under vLLM,
or a non-Llama GGUF under `llama-server`.

```json
{
  "alias": "qwen3-coder",
  "path": "openai:http://127.0.0.1:8080/v1?model=qwen3-coder-30b&timeout_ms=180000"
}
```

- The upstream model name defaults to the binding alias; `model` overrides it,
  so a mesh alias need not match the upstream's own name.
- `timeout_ms` bounds each request (default 5 minutes, ceiling 1 hour).
- `max_new_tokens` sets this binding's generation budget (default 2048, ceiling
  `UPSTREAM_MAX_NEW_TOKENS` = 8192).
- Credentials never appear in the binding. The runtime reads a bearer token from
  `TACHYON_UPSTREAM_API_KEY_<ALIAS>` (alias upper-cased, non-alphanumerics
  folded to `_`), falling back to `TACHYON_UPSTREAM_API_KEY`. With neither set,
  the request goes out unauthenticated — the usual case for a `llama-server` on
  a trusted mesh link.

### Upstream work does not enter the batch scheduler

Upstream bindings run on the `Network` lane, which has no dispatcher thread at
all. They are admitted by a counting semaphore instead — `UpstreamAdmission`,
default 32 concurrent round trips, overridable with
`TACHYON_UPSTREAM_MAX_CONCURRENCY`, with a bounded 30-second wait after which
the node sheds the request rather than growing an invisible backlog.

The batch scheduler exists to amortise one GPU forward pass over co-batched
sequences. An HTTP round trip gains nothing from that and loses two things to
it. The dispatcher thread is shared with every local model on the node, so a
slow provider stalls unrelated local inference; and the batch barrier makes each
caller wait for the slowest peer in its batch, so a batch of 32 charged the last
caller the sum of all preceding upstream latencies. What upstream work actually
needs is a cap on how much runs at once — which is what the gate is, held for
the whole interaction (for a stream, its entire lifetime, not just its first
byte) and shared by the buffered, streaming, and embedding paths so the cap is a
property of the node rather than of one entry point. A burst of
`/v1/embeddings` cannot open unbounded sockets while `/v1/chat/completions`
stays gated.

Concurrency comes from there being many caller threads, each holding one permit
— not from fanning one call out, which is why the backend now runs its inputs
sequentially and the scoped-thread fan-out is gone.

A permit is only as good as its release, so a disconnected client has to reach
the backend. When the guest drops its `token-stream`, the host's channel send
fails, and that is reported back down as `StreamControl::Stop` rather than
ignored: the upstream reader abandons the response instead of draining it to
`[DONE]`. Without that signal an abandoned SSE stream would hold its permit
until the binding's timeout — up to an hour — and a handful of them would spend
the node's whole outbound capacity on readers that left. The local decode loops
honour the same signal beside their deadline check, for the same reason a
deadline exists: finishing an answer nobody will read only occupies the slot.
Speculative decoding checks it twice — once per round and once inside the
acceptance loop — because one round verifies up to `draft_tokens` proposals, so
checking only at the top would still run that many target forward passes after
the client had gone.

A failed send is not the only signal, because it only fires when there is
something to send. A sink also answers `is_live()` — backed by a flag the
`token-stream` resource clears on drop — and the upstream reader asks it once
per SSE frame, so a stream of role-only openings, usage frames or keep-alives
cannot run to completion for a client that has already left.

One window stays open: the read that waits for the upstream's *first* frame is
uninterruptible, so a client that leaves before any frame arrives is noticed
only when one does. Closing it needs a cancellable read, which the blocking HTTP
client does not expose; until then the exposure is bounded by the binding's
`timeout_ms` (and by a caller's `max_generation_ms`, which tightens it).

One consequence is worth stating because it is easy to get wrong: with nothing
enqueued on the `Network` lane, its scheduler queue depth would be permanently
zero, and the mesh QoS admission check reads exactly that number to decide
whether to spill traffic to a peer. So `queue_tier_snapshot(Network)` reports
the gate's **waiting** count — callers blocked on a permit — not its in-flight
count. An in-flight request is being served; a waiting one is work this node
cannot start, which is what queue depth meant on the local lanes.

### Tool calling through an upstream

This is the one place the upstream path is meaningfully *better* than the native
one, and it matters for agentic clients like OpenCode.

The native runtime has no tool-aware chat template: `tools` never reach the
model, and `guest-openai` recovers tool calls by parsing the model's text output
with a `tool_call_parser`. An upstream server does have one, so `tools` and
`tool_choice` are forwarded verbatim and the upstream applies its own template
(and, with vLLM, constrained tool-call generation).

A tool-call response carries `content: null`, and the calls travel on their own
channel rather than inside the text. The accelerator interface carries a
`tool-call` record — an optional provider id, a function name, a JSON argument
string — on both the buffered `generation` and the streaming `stream-event`, so
`guest-openai` receives them as fields and does the OpenAI encoding itself.

That the host does *not* encode them is the point. What the WIT carries is what
the model said; what OpenAI's `tool_calls` array looks like is the gateway's
business, and a consumer speaking a different wire format never has to undo an
OpenAI encoding first. **No client-side configuration is required either**:
recovering a structured call no longer depends on parser selection, which comes
from a nonstandard request option or a guess at the model name — so a standard
client offering tools would previously get no parser at all, or a tagged one
that cannot read JSON, and the call would come back as literal assistant prose.

`token-stream` reports the finish reason on the same terms as `usage`: known
only once `next` has returned `none`, absent when the backend did not say. It
is a separate accessor for the same reason the counts are — both are known only
at the end, and sending either as a final fragment would make them
indistinguishable from model output. A truncated streamed completion was
otherwise reported as `stop`, which is how a client comes to run half a
function.

The streaming and buffered paths must agree on the message, not just on the
calls: concatenating the content deltas yields exactly what the buffered path
returns. Whitespace is where that is easy to lose — the buffered parser removes
the tool-call region and trims what is left, so the gate withholds whitespace at
its tail until it knows whether an opener follows. If none does, the caller's
reconciliation releases it; if one does, it was never the message.

`length` outranks `tool_calls` when both apply. A model that exhausts its budget
*while emitting a call* returns a partial `tool_calls` entry alongside
`finish_reason: "length"`; reporting `tool_calls` there tells the client the call
is ready and invites it to dispatch truncated arguments.

Streamed tool calls arrive as `delta.tool_calls` fragments with no content at
all — name first, `arguments` in pieces after it. They are reassembled by
fragment `index` and emitted as `stream-event::tool-call` once the stream ends,
because a call is only dispatchable complete; dropping them would make the
request look like a model that answered with silence.

The structured channel is also what keeps time-to-first-token on tool-enabled
requests. With calls confined to the text channel, prose had to be accumulated
until the stream finished — a whole-output JSON envelope emitted *after* streamed
prose is unparseable — so merely offering tools cost the entire generation in
latency, including on the requests that never called anything. Content now
streams unconditionally.

### Versioning the accelerator interface

The interface is `tachyon:accelerator@2.0.0`. The bump is **major** rather than
minor because `compute`, `embed`, `compute-stream` and `token-stream.next` all
changed shape — their error type became `generation-error`, and `next` yields a
`stream-event` instead of a string.

That distinction is load-bearing, not bookkeeping. Wasmtime resolves a component
import by semver-*compatible* name, so a component built against 1.1.0 would
have bound to a 1.2.0 host and then failed instantiation with a structural type
mismatch — an error pointing at the linker rather than at the version. Under a
major bump the same component fails with an unsatisfied import, which says what
actually happened. Preserving the 1.1 signatures was the alternative, but it
means keeping a text-channel `next` alongside the structured one, and with it
the whole-output tool-call envelope and the buffering it forced — paying in
latency, permanently, for a compatibility nobody is currently using.

### Failures carry the upstream's status

`compute`, `compute-detailed`, `embed`, `compute-stream` and `token-stream.next`
fail with a `generation-error`: a message, plus `upstream-status` when the
failure *was* a remote HTTP response. Local failures — a decode error, an
unknown alias, a rejected request — leave it absent rather than inventing a
status.

The status is what makes the relay honest, because the client's own behaviour
depends on it. `guest-openai` maps it:

| upstream status | relayed as | why |
|---|---|---|
| 429 | 429 `rate_limit_error` | the client's backoff has to engage |
| 400/404/405/409/413/422 | 400 `invalid_request_error` | the provider rejected what we forwarded, which reflects the caller's own parameters |
| 502/503/504 | relayed unchanged | a transient gateway failure, retryable as itself |
| 401/403, anything else | 502 `server_error` | an upstream credential failure is *this node's* misconfiguration; relaying a 401 would send the client chasing its own key |
| absent | 500 `server_error` | nothing remote to attribute |

A streamed request cannot use any of that directly: its status line is already
on the wire by the time generation starts. The same error body is written as a
final SSE frame instead, followed by `[DONE]`, which is where an OpenAI client
reads a mid-stream failure anyway.

Collapsed into one opaque string, a 429 the client should back off from and a
400 it must never retry reach it identically — and typically as a server fault
it can only retry blindly.

### Streaming tool calls end to end

`guest-openai` recovers tool calls on the streaming path too, not just the
buffered one, so an agentic client receives real `delta.tool_calls` and
`finish_reason: "tool_calls"` instead of raw text in its transcript. A call the
*host* reported structurally is adopted as-is and always wins over anything
parsed out of the text — the backend received it as fields, so re-reading the
text could only guess worse. The gate below is for the other case: a local model
whose chat template emits calls as text.

Buffering the whole generation before parsing would be the simple way to do
that, but it destroys time-to-first-token for every request that merely *offers*
tools — which, for an agentic client, is every request. So `StreamingContentGate`
forwards content as it arrives and stops the moment a tool-call opener appears;
from there everything is held for the buffered parser, because the tool-call
region is not content and must not leak into the transcript. Bytes near the tail
are held back so an opener split across two fragments is still matched — the
same trick the host decode loop uses for stop sequences.

Openers are per-parser. The `json` parser is *anchored*: `parse_json_tool_calls`
requires the whole output to be one JSON value, so a `{` anywhere but the start
cannot begin a call and must not stop prose from streaming. The tagged parsers
(`<tool_call>`, `[TOOL_CALLS]`) scan anywhere, because those models routinely
emit prose and then a call.

One deliberate asymmetry: the gate streams raw text, while the buffered parser
trims its content. A streamed chunk cannot be un-sent, so the handler reconciles
afterwards by prefix rather than by byte offset — if what was streamed is not a
prefix of the parsed content, no tail is emitted at all, because duplicating
text in the transcript is worse than omitting a trailing fragment.

The host's JSON generation request maps onto the chat-completions body:
`messages` is forwarded verbatim (the upstream applies its own chat template,
since the model lives there), `prompt` becomes a single user turn,
`max_new_tokens` becomes `max_tokens`, and `json_schema` becomes an OpenAI
`response_format`. Streaming is real SSE passthrough — one token per content
delta — so time-to-first-token survives the extra hop. LoRA adapters are
rejected: adapter injection has no wire representation here.

A generation budget is always sent, so an absent `max_new_tokens` cannot leave
the upstream's own (possibly unlimited) default in charge. The bound is the
binding's own, **not** the native runtime's `HOST_MAX_NEW_TOKENS`, because the
two limits protect different resources: the native cap bounds this host's decode
loop, where every extra token is a forward pass and more KV cache in local VRAM,
while an upstream generation costs this node one open HTTP connection and spends
the *remote* server's resources, which enforces its own context window and
queueing.

`guest-openai` forwards an omitted `max_tokens` as an omission rather than
substituting its own default, so the budget a binding advertises is the one that
actually applies on the public `/ai/v1` route.

So upstream bindings default to 2048 tokens, are capped at
`UPSTREAM_MAX_NEW_TOKENS` (8192), and can be tuned per alias with
`?max_new_tokens=`. The per-binding override is itself bounded, so a typo cannot
make a request effectively unlimited.

The native cap is `HOST_MAX_NEW_TOKENS` = **4096**, raised from 256 (which
truncated an agentic response mid-function) and `DEFAULT_MAX_NEW_TOKENS` = 1024,
raised from 64. 4096 is where the costs that actually scale stay reasonable on
the targeted hardware: KV cache grows linearly and is bounded by the context
window anyway, while the contiguous cache path's quadratic memory-traffic term
is still small next to inherent decode cost at that length.

### URL constraints

The base URL is validated structurally at load, not on first request. Rejected:
a missing or inferred host, embedded credentials (`https://user:pass@host/v1` —
reqwest would turn userinfo into a Basic auth header, bypassing the
environment-only credential contract, and `base_url` is echoed in telemetry and
errors), and `#fragments` (never sent on the wire, so the route suffix would be
silently lost). Query values are percent-decoded through the URL API, so a
`?model=Qwen%2FQwen3` written by any standard URL builder reaches the upstream
as `Qwen/Qwen3`.

A batch is dispatched concurrently, one scoped thread per request, and results
are returned per input. Both matter here and not for local runtimes: these are
independent network round trips on the accelerator dispatcher thread shared by
every CPU-resident model, so running them sequentially would make the last
caller wait for the sum of all preceding upstream latencies and block unrelated
local inference meanwhile; and an upstream failure is usually about one request
(a rejected prompt, a 400), so collapsing the batch into a single `Result` would
fail every co-batched caller and discard responses already received.

Every read is bounded — response bodies, error excerpts, whole SSE streams, and
individual SSE frames — and a stream that ends before its `[DONE]` sentinel is
reported as a truncated generation rather than a successful one, so an upstream
that restarts mid-response cannot hand the caller silently truncated code.
Embedding components that do not narrow to a finite `f32` are rejected instead
of becoming infinities that turn downstream cosine similarities into NaN.

Binding validation is offline, so a node still boots when its upstream is down;
an unreachable server is a per-request error naming the endpoint and status, and
an error body is never returned as generated text. Because no weights are
resident locally, an upstream binding reports no local accelerator residency
whatever `device` it declares.

## Cross-Node Model Placement Watchlist

Tachyon currently optimizes for homelab/edge deployments where the target model
fits within one RTX-class node. Multi-GPU model parallelism is active only as a
single-node placement concern. `parallel.rs` contains the NCCL all-reduce path
for CUDA tensor-parallel ranks and a minimal TCP stage transport that can move
pipeline activations over a real socket, but Tachyon does not treat placement of
one live model across multiple machines as an active product requirement.

Keep production cross-machine placement of one live model in watchlist status
until a roadmap model exceeds the aggregate VRAM capacity of a single target
node, for example a 100B+ unquantized checkpoint or an unusually long-context
deployment. The existing NCCL TCP bootstrap and `StageTransport` socket
primitives remain valid groundwork, but they are not by themselves a product
requirement to orchestrate one forward pass across machines. Until the roadmap
trigger is met, scale horizontally by overflowing whole requests to peers that
already host the model. On reactivation, reassess how far
`discover_cluster_topology()`, `parallel.rs`, and the TCP bootstrap cover the
target topology before sizing placement, NUMA binding, peer failure during
forward, and production orchestration work.

## PagedAttention Status

The pinned `astorise/candle` fork is consumed through a Tachyon release tag
(bumped as the fork gains capabilities Tachyon depends on — see
`core-host/Cargo.toml` for the current pin), not a raw commit rev, so Renovate
can track the git ref. Each fork refresh should rebase the fork on the selected
upstream Candle commit, run the Candle/Tachyon AI inference checks, publish a new
`tachyon-v<upstream-version>-<N>` tag, and update `core-host/Cargo.toml` plus
`Cargo.lock` together. The fork includes Candle's paged flash-attn API
(`flash_attn_varlen_paged_windowed`) and an additive `Cache::set_paged_kv`/
`paged_kv` per-layer seam in `candle-transformers::models::llama` (tag
`tachyon-v0.11.0-3`, [astorise/candle#8](https://github.com/astorise/candle/issues/8)).

`hardware_strategy.paged_attention: true` is enabled for a **Llama checkpoint
on a CUDA device** (see "Single-Device GPU Execution" above for that
baseline): `core-host/src/ai_inference/paged_kv.rs` owns a `PagedBlockPool`
(fixed-size free-list allocator) and a per-request `SequenceBlockTable`;
`candle_llm_runtime.rs` allocates one `(key_cache, value_cache)` tensor pair
per transformer layer, sized from real NVML free-VRAM telemetry (a fixed
heuristic — 50% of free VRAM remaining after weight load, with a floor of
one full-length sequence at the checkpoint's `max_position_embeddings` —
not yet a configurable `hardware_strategy` knob), and attaches
`cache.set_paged_kv(..)` for every layer on every forward step. Every other
architecture, and any non-CUDA device or build without `candle-cuda`, still
returns the existing typed unsupported-model error instead of silently
falling back to the contiguous per-request KV cache.

**BF16, not F32**: `candle-flash-attn`'s kernels only support F16/BF16 (an
industry-standard constraint of fused attention kernels, not specific to
Tachyon), so a paged-attention Llama deployment loads its weights and KV
cache in BF16 instead of the contiguous path's F32. Generation is a real,
deterministic decode (repeating the same greedy request yields identical
output), but is not expected to be bit-identical to the F32 contiguous path.

Not yet supported in combination with paged attention: speculative decoding
(falls back to plain generation rather than erroring, since verification
would otherwise dtype-mismatch against the BF16 model) and continuous
batching across concurrent sequences (each request gets its own block table
today; the shared pool is reused across requests, but only one sequence is
in flight per forward pass). LoRA adapters are unaffected either way — they
already reload an independent CPU/F32 model per request.

Model bindings can opt in with:

```json
{
  "hardware_strategy": {
    "paged_attention": true
  }
}
```

## FlashInfer Decode Attention Status

The pinned `astorise/candle` fork carries an additive `use_flashinfer_attention`
seam on `candle_transformers::models::llama::Config` (mirroring the existing
`use_flash_attn` flag), wired to the optional `candle-flashinfer-kernels`
crate's `flashinfer_decode_attention`.

A Llama model binding on a CUDA device with the `candle-flashinfer` Cargo
feature compiled in can opt in with:

```json
{
  "hardware_strategy": {
    "flashinfer_attention": true
  }
}
```

The decode step (one query token per sequence) then attends via
`candle_flashinfer_kernels::flashinfer_decode_attention` against the
pre-`repeat_kv` contiguous KV cache instead of the dense matmul+softmax (or
`flash_attn`) path; prefill is unaffected. Unlike `paged_attention`, this needs
no dtype switch — a flashinfer-attention deployment stays on the same F32 path
as the plain dense Llama path, since the kernel supports F32/F16/BF16 with no
block-size or head-dim alignment requirement. `flashinfer_attention` cannot be
combined with `paged_attention` in the same deployment (they select different
decode-attention kernels over different KV cache layouts) — that combination
is rejected with a typed error rather than silently picking one. Every other
combination (non-Llama architecture, non-CUDA device, or a build without
`candle-flashinfer` compiled in) keeps the existing typed rejection. See
`openspec/changes/wire-flashinfer-decode-attention`.

## CUDA Graph Decode Status

The fork carries `candle_core::CudaGraph` (capture/replay) plus two additive
seams a captured decode step needs: `Cache::set_decode_position` (rotary
embeddings read a persistent, in-place-updatable device tensor instead of the
host-side `narrow(0, index_pos, seq_len)` a replayed graph would otherwise
silently bake in) and `Cache::set_paged_kv_decode_slot` (the KV-cache scatter
destination is likewise a persistent device tensor the caller updates in
place, instead of `PagedKvCache::write_new_kv` deriving it via a blocking
device-to-host block-table readback — itself not capturable at all).

A Llama model binding on a CUDA device can opt in with both flags together:

```json
{
  "hardware_strategy": {
    "paged_attention": true,
    "cuda_graph_decode": true
  }
}
```

`cuda_graph_decode` requires `paged_attention: true` (typed error naming the
dependency when missing) — the contiguous KV cache's per-step reallocation
via `Tensor::cat` is incompatible with CUDA graph replay, and the
pre-allocated `PagedKvCache` layout is the only candidate this runtime has.
Once both flags are set on a supported build, `CudaGraphDecodeSession` runs
the steady-state decode step (post-prefill, one token at a time) through a
real captured/replayed graph: the block-table/seqlens/position/decode-slot
tensors are sized once at their full maximum width and updated only via
`Tensor::slice_set`, `model.forward`'s `index_pos` argument becomes
irrelevant once both position/decode-slot seams are attached (the real
position flows through those device tensors instead), and every decode step
after the first captured one is a single `CudaGraph::replay()` call. Prefill,
LoRA adapters, and speculative decoding are unaffected (prefill always uses
the existing uncaptured path — chunk shapes vary chunk to chunk; speculative
decoding already falls back to plain generation for any paged-attention
target/draft). Every other combination (non-Llama architecture, non-CUDA
device, or `cuda_graph_decode` without `paged_attention`) keeps the existing
typed rejection.

A capture, warm-up, and however many replays a request needs are correct on
real GPU hardware, with output identical to the non-captured paged-attention
path for the same prompt. A second independent request against the same
loaded runtime is also proven: PR #367's `cuda-quality` job runs the two
requests in sequence and asserts identical greedy output. This relies on
`astorise/candle` `tachyon-v0.11.0-10` and its matching `cudarc` patch, which
keep event tracking paused for the graph lifetime and safely tear down
graph-owned CUDA allocations before another request creates a cache.

## Speculative Decoding Status

Model bindings can opt into greedy draft/verify decoding with a local draft
model directory:

```json
{
  "hardware_strategy": {
    "speculative_draft_model_path": "/models/tiny-draft",
    "speculative_draft_tokens": 4
  }
}
```

The draft model proposes a bounded token window and the target model verifies
each token before emission. This first path is exact for greedy single-prompt
generation; sampling, constrained decoding, tokenizer-incompatible drafts, and
multi-prompt batches fall back to the existing target-only decode path.
