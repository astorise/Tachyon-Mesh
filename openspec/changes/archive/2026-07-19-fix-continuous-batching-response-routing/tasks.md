## 1. Root cause

- [x] 1.1 Trace `process_batch` → `CandleModel::run_mock_batch` →
  `BackendModel::execute`/`execute_with_adapter` and confirm every real-model
  arm (`Qwen35Moe`, `ModelOptNvfp4`, `TextGeneration`) forwards the whole
  batch's prompts to a runtime method that only ever decodes the first one
  and returns a single combined output, which `process_batch` then cloned
  across every job in the batch (design.md Context).
- [x] 1.2 Confirm `scheduler_batches_concurrent_requests_for_same_alias`
  cannot detect this: it sends the identical prompt from every concurrent
  thread to a mock model with a fixed response, so contamination and
  correctness are observationally identical.

## 2. Fix

- [x] 2.1 Change `BackendModel::execute`/`execute_with_adapter` to
  `Result<Vec<Vec<u8>>>` (one output per input, in order) instead of
  `Result<Vec<u8>>` (design.md Decision 1).
- [x] 2.2 Update `CandleBackendModel::execute`'s `Qwen35Moe`, `ModelOptNvfp4`,
  `TextGeneration` (incl. speculative), and `Vendor` arms to loop over each
  input and decode it independently via a single-element slice per call,
  instead of forwarding the whole batch to a single runtime call
  (design.md Decision 2). Mock arm returns one copy of the fixed response
  per input.
- [x] 2.3 Update `CandleBackendModel::execute_with_adapter`'s
  `TextGeneration`/`ModelOptNvfp4` arm the same way.
- [x] 2.4 Update the trait's default `stream_text` and
  `CandleBackendModel`'s `Mock`/`Vendor` `stream_text` arms (the only other
  callers of `execute`) for the new `Vec<Vec<u8>>` return shape. Confirmed
  `stream_text` itself is only ever called with a single-element input slice
  (its sole call site), so it requires exactly one output back.
- [x] 2.5 Update `CandleModel::run_mock_batch` to return `Result<Vec<Vec<u8>>>`.
- [x] 2.6 Update `process_batch` to zip `outputs[i]` with `batch[i]`'s own
  response channel, and to fail the whole batch with a descriptive error
  (rather than indexing incorrectly) if `outputs.len() != batch.len()`
  (design.md Decision 1).

## 3. Tests

- [x] 3.1 Add `scheduler_routes_distinct_concurrent_prompts_to_their_own_response`:
  real Candle Llama fixture, three distinct prompts (`hello`/`tachyon`/`mesh`
  — the fixture's full 4-word vocab minus `<unk>`), reference outputs
  computed sequentially first, then re-issued concurrently via the same
  `Barrier` pattern as the existing scheduler test; asserts each caller's
  output matches its own prompt's reference.
- [x] 3.2 Verified this test fails against the pre-fix broadcast behavior:
  temporarily reverted `process_batch`'s routing to
  `outputs.into_iter().next()` cloned to every job, confirmed the new test
  failed with a clear "received another concurrent request's output"
  assertion mismatch, then restored the fix and confirmed it passes.
- [x] 3.3 Regression: `cargo test -p core-host --features ai-inference
  ai_inference::` — 185/185 passing (184 pre-existing + 1 new).
- [x] 3.4 `cargo clippy --workspace --all-targets --features
  core-host/ai-inference -- -D warnings -D clippy::unwrap_used` — clean.
- [x] 3.5 `RUSTFLAGS="-D dead_code" cargo check -p core-host --features
  ai-inference` — clean.
- [x] 3.6 `cargo fmt --all -- --check` — clean.

## 4. Docs

- [x] 4.1 `openspec/specs/ai-inference/spec.md`: new scenario under the
  existing continuous-batching requirement (this change's spec delta).
- [x] 4.2 `CHANGELOG.md` entry describing the bug and fix.
- [x] 4.3 Noted in issue #312: the "continuous batching" item's scheduling
  infrastructure already existed and was already spec-compliant on paper;
  this change closes the gap between that spec and the implementation. True
  fused multi-sequence GPU batching (throughput) remains open as a distinct,
  larger follow-up (design.md Non-Goals) — not required to consider this
  item done from a correctness standpoint.
