## Why

Discovered while starting issue #312's remaining "continuous batching" step:
the scheduler infrastructure this step names already exists and is already
specified (`openspec/specs/ai-inference/spec.md`'s "Inference requests are
continuously batched by the host" requirement, scenario "Compatible inference
requests are active together" — "routes each generated response back to the
correct caller"). The code was not actually compliant with that scenario.

`process_batch` (`core-host/src/ai_inference.rs`) collects one `input` from
each `InferenceJob` sharing a decode batch, calls
`model.run_mock_batch(&inputs, adapter)`, and — despite the name — this is a
real, non-mock dispatch: `CandleBackendModel::execute` forwards the full
`inputs` slice to the underlying Candle runtime (`runtime.generate(&prompts)`,
`target.generate_speculative(&prompts, ...)`, etc.). Every one of those
runtime methods parses all prompts but only ever decodes `parsed.first()` and
returns one combined output. `process_batch`'s success arm then did
`batch.iter().map(|_| Ok(output.clone())).collect()` — cloning that single
output across every job in the batch. Concurrent different-prompt requests
to the same model alias (`DEFAULT_BATCH_SIZE = 32` makes multi-request
batches the default, not an edge case) could silently receive each other's
generated text.

This was not caught by `scheduler_batches_concurrent_requests_for_same_alias`
because that test sends the *identical* prompt from all 8 threads to a mock
model with a fixed response — correctness and contamination look identical
in that setup.

## What Changes

- `BackendModel::execute`/`execute_with_adapter` (`core-host/src/ai_inference.rs`)
  change from `Result<Vec<u8>>` (one output for the whole call) to
  `Result<Vec<Vec<u8>>>` (exactly one output per input, in order).
- `CandleBackendModel`'s implementation of both methods (`Qwen35Moe`,
  `ModelOptNvfp4`, `TextGeneration`/speculative, `Vendor`, `Mock` arms) is
  changed to loop over each input and decode it independently — a single
  extra prompt no longer causes the runtime's own "exactly one prompt"
  internal decode logic to silently drop it or bail. This is a correctness
  fix via sequential per-request processing, not a GPU-level fused multi-
  sequence forward pass (see Non-Goals).
- `process_batch` routes `outputs[i]` back to `batch[i]`'s own response
  channel instead of cloning `outputs[0]` to every job, and treats a
  outputs/batch length mismatch as a hard backend error rather than
  silently misrouting.
- Adds `scheduler_routes_distinct_concurrent_prompts_to_their_own_response`,
  a regression test using a real Candle Llama fixture with three distinct,
  concurrently-issued prompts, asserting each caller gets back its own
  prompt's output. Verified this test fails against the old broadcast
  behavior and passes against the fix (see `tasks.md`).

## Capabilities

### New Capabilities
(none)

### Modified Capabilities
- `ai-inference`: the "Inference requests are continuously batched by the
  host" requirement gains an explicit scenario naming the per-request
  output-isolation guarantee the existing "routes each generated response
  back to the correct caller" wording already implied but did not test —
  and the implementation is brought into compliance with it.

## Impact

- `core-host/src/ai_inference.rs`: `BackendModel` trait signatures,
  `CandleBackendModel::execute`/`execute_with_adapter`, the default
  `stream_text` fallback, `CandleModel::run_mock_batch`, and `process_batch`.
- `openspec/specs/ai-inference/spec.md`: new scenario under the existing
  continuous-batching requirement.
- `CHANGELOG.md`: entry describing the bug and fix.
- Out of scope: true fused multi-sequence GPU batching (a single forward
  pass over a batch dimension > 1 for throughput). This change fixes
  *correctness* — every request gets its own right answer — via sequential
  per-prompt processing inside one scheduler batch; it does not change how
  many GPU forward passes a batch of N requests costs. Real throughput
  gains from batched GPU execution are a distinct, larger follow-up (see
  `design.md` Non-Goals) that would build on paged attention's already
  batch-shaped tensor helpers (`build_block_table_tensor`/
  `build_cumulative_seqlens_tensor` already accept `&[&SequenceBlockTable]`).
