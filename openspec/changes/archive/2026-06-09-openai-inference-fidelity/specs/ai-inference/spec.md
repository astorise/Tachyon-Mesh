## ADDED Requirements

### Requirement: Candle LLM generation supports configurable sampling
The Candle LLM runtime SHALL select tokens according to per-request sampling
parameters — `temperature`, `top_p` (nucleus), and `seed` — while remaining
deterministic by default. A `temperature` that is absent or `<= 0` SHALL produce
deterministic greedy decoding independent of the seed. A sampled request that
omits `seed` SHALL fall back to a fixed default seed so it stays reproducible.

#### Scenario: Greedy by default
- **WHEN** a generation request omits `temperature` or sets it to `0`
- **THEN** the runtime decodes greedily (argmax)
- **AND** two runs of the same prompt produce identical output

#### Scenario: Seeded sampling is reproducible
- **WHEN** a generation request sets `temperature > 0` and a fixed `seed`
- **THEN** the runtime samples from the temperature-scaled distribution
- **AND** two runs of the same prompt and seed produce identical output

#### Scenario: Nucleus filtering is bounded
- **WHEN** a sampled request sets `top_p` inside the open interval `(0, 1)`
- **THEN** the runtime restricts sampling to the smallest set of tokens whose
  cumulative probability reaches `top_p`
- **AND** a `top_p` of `1.0` (or absent) disables nucleus filtering

### Requirement: Chat requests render the model's own chat template
When a generation request carries structured `messages`, the runtime SHALL
render them into the prompt using the checkpoint's own `chat_template` loaded
from `tokenizer_config.json`, including the special tokens it references and an
appended generation prompt. When the checkpoint ships no template, the runtime
SHALL fall back to a generic rendering that ends on an open assistant turn. A
request MAY instead supply a raw `prompt`, which is used verbatim.

#### Scenario: Model template drives rendering
- **GIVEN** a checkpoint whose `tokenizer_config.json` declares a `chat_template`
- **WHEN** a request supplies structured `messages`
- **THEN** the runtime renders the conversation with that template and its
  special tokens, ready for the assistant to continue

#### Scenario: Generic fallback without a template
- **GIVEN** a checkpoint with no `chat_template`
- **WHEN** a request supplies structured `messages`
- **THEN** the runtime renders a generic `role: content` prompt ending on an
  open assistant turn
- **AND** generation still runs

#### Scenario: Raw prompt bypasses templating
- **WHEN** a request supplies a raw `prompt` and no `messages`
- **THEN** the runtime tokenizes the prompt verbatim

### Requirement: Generation honours stop sequences
The runtime SHALL accept a bounded list of `stop` strings and halt generation as
soon as any of them appears in the decoded text, returning the text up to (and
excluding) the earliest match. An empty or oversized stop entry SHALL be ignored.

#### Scenario: Output is trimmed at the earliest stop
- **WHEN** a request sets one or more `stop` strings
- **AND** the decoded text reaches a stop sequence
- **THEN** generation halts
- **AND** the returned text excludes the stop sequence and anything after it

#### Scenario: Stop list is bounded
- **WHEN** a request supplies empty, oversized, or excessively many stop strings
- **THEN** the runtime filters and caps the list before decoding

### Requirement: The runtime streams decoded fragments incrementally
The runtime SHALL provide a streaming generation path that emits each newly
decoded text fragment as it is produced, such that the concatenation of all
fragments equals the buffered generation output for the same request. While
streaming with stop sequences, the runtime SHALL hold back the trailing text
that could begin a stop match until a further token confirms it is safe to emit.

#### Scenario: Streamed fragments reconstruct the buffered output
- **WHEN** the same request is run buffered and streamed
- **THEN** the streamed path emits one or more fragments
- **AND** their concatenation equals the buffered output byte-for-byte

#### Scenario: Non-text backends fall back to a single fragment
- **WHEN** a streaming request targets a backend that cannot decode
  incrementally (mock or NVFP4)
- **THEN** the runtime emits the entire output as one fragment

### Requirement: The accelerator exposes a streaming compute primitive
The `tachyon:accelerator/cpu` interface SHALL provide a `compute-stream`
function returning a `token-stream` resource whose `next` yields decoded text
fragments until it returns `none` (generation complete). The streaming path
SHALL enforce the same sealed-alias scope and accelerator-handle checks as the
buffered `compute`, and the new function SHALL be additive so existing guests
are unaffected.

#### Scenario: Streaming a sealed model
- **GIVEN** a guest holding an accelerator handle for a sealed model alias
- **WHEN** it calls `compute-stream` and pulls with `next`
- **THEN** the host yields decoded fragments as they are produced
- **AND** `next` returns `none` once generation completes

#### Scenario: Streaming respects the scope gate
- **WHEN** a guest calls `compute-stream` for a handle it does not hold, or for
  an alias not sealed for its route
- **THEN** the host rejects the call with an error, exactly as `compute` does
