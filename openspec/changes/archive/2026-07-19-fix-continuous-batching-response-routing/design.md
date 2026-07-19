## Context

Issue #312's remaining "continuous batching" item assumes there's scheduling
work left to build. There isn't — `AcceleratorScheduler`, `InferenceJob`,
`select_active_batch` (groups jobs by `alias`+`adapter_id` and phase),
`run_continuous_step`, and QoS-aware admission (`PrioritizedInferenceJob`,
`age_waiting_jobs`) already exist and already match the spec's "Inference
requests are continuously batched by the host" requirement. What's missing is
correctness at the one seam where a batch of N independent requests becomes
N independent *responses*: `process_batch` → `CandleModel::run_mock_batch` →
`BackendModel::execute`.

Tracing that seam:

1. `process_batch` collects one `SharedInputTensor` per job into `inputs`.
2. `run_mock_batch` forwards the whole slice to `execute`/`execute_with_adapter`.
3. Every real-model arm of `CandleBackendModel::execute` (`Qwen35Moe`,
   `ModelOptNvfp4`, `TextGeneration`) collected `inputs` into a `prompts: Vec<&[u8]>`
   and called a `CandleLlmRuntime`/`Qwen35MoeRuntime` method with the *whole*
   slice — `runtime.generate(&prompts)`, `target.generate_speculative(&prompts, ...)`.
   Every one of those methods parses all prompts (`parsed = prompts.iter().map(parse_request)...`)
   but decodes only `parsed.first()` and returns one `Vec<u8>`. This validated
   the whole batch but only ever executed request 0.
4. `Vendor`'s arm didn't even collect all inputs — `inputs.first()` only,
   silently dropping the rest.
5. `process_batch`'s success arm cloned that one `Vec<u8>` across every job:
   `batch.iter().map(|_| Ok(output.clone())).collect()`.

Net effect: batch of N different prompts to the same alias → job 0's prompt
gets decoded once → every job in the batch (including jobs whose prompt was
never even looked at) receives job 0's output. `DEFAULT_BATCH_SIZE = 32` is
the default active-set size, so this is not a rare timing edge case — any two
concurrent requests to the same model alias that get admitted into the same
decode step are exposed to it.

Why existing tests missed it: `scheduler_batches_concurrent_requests_for_same_alias`
sends the byte-identical prompt (`b"hello"`) from all 8 threads to a mock
model with a fixed response regardless of input. Contamination and
correctness produce the same observable result in that setup.

## Goals / Non-Goals

**Goals:**
- Every request in a scheduler batch receives the output generated from *its
  own* input, never another request's.
- No change to scheduling policy (admission, QoS ordering, batch-key
  compatibility) — this is purely about response routing once a batch is
  already formed.
- Verifiable on CPU, without GPU hardware: this is response-routing plumbing,
  not kernel behavior.

**Non-Goals:**
- True fused multi-sequence GPU batching — one forward pass processing a
  batch dimension > 1 for throughput. This change processes each request in
  a batch sequentially (N calls into the existing single-prompt decode path,
  same per-request output as running it alone), which fixes correctness but
  does not reduce the number of forward passes N requests cost. That's a
  distinct, materially larger change: it needs per-layer batched KV state
  (paged attention's `PagedKvCache` is already shaped for this — its
  block-table/seqlens builders already accept `&[&SequenceBlockTable]`,
  plural — but the dense/contiguous-cache path is not), a batched sampler,
  and its own real-hardware validation cycle, matching the pattern every
  other accelerator item in issue #312 has followed. Flagging as a follow-up
  rather than attempting both at once.
- Changing `CandleLlmRuntime::generate`/`generate_streaming`/`Qwen35MoeRuntime::generate`'s
  own signatures. They still accept `&[&[u8]]` and still only decode the
  first element — that partial-batching shape predates this change and is
  untouched. The fix is entirely in how `ai_inference.rs` *calls* them: once
  per input, with a single-element slice, instead of once with the whole
  batch. Revisiting those runtime-layer signatures is part of the true-fused-
  batching follow-up above, not this fix.

## Decisions

### 1. `BackendModel::execute`/`execute_with_adapter` return `Result<Vec<Vec<u8>>>`
One `Vec<u8>` per input, same order as `inputs`. A whole-call `Err` still
means "the whole batch failed" (unchanged semantics — `process_batch`
already distributes a shared error to every job on failure, which remains
reasonable: today's per-prompt calls are independent enough that isolating a
single bad prompt's failure from its batch-mates is a further refinement, not
required to fix the contamination bug). `process_batch` additionally treats
`outputs.len() != batch.len()` as a hard backend error (defensive: exactly
the failure shape this bug produced, now caught structurally instead of
silently indexing wrong).

### 2. Fix by looping at the `ai_inference.rs` call sites, not the runtime layer
Every `CandleLlmRuntime`/`Qwen35MoeRuntime` generation method already
implements correct single-prompt decoding — `generate(&[prompt])` for one
prompt was always correct. The bug was calling it once with N prompts
instead of N times with one prompt each. Looping at the call site:
- Reuses proven, already-tested single-prompt logic verbatim.
- Needs no changes to `candle_llm_runtime.rs` or `qwen35_moe_runtime.rs`.
- Naturally fixes `Qwen35Moe`'s arm too: its runtime already hard-rejects
  `prompts.len() != 1` (`bail!("... currently accepts exactly one prompt
  per decode")`), so today a heterogeneous batch would error the whole
  batch; called once per prompt instead, each individual call satisfies
  that invariant and now actually generates for every request.
- Naturally fixes `Vendor`'s arm: loops `runtime.execute(...)` once per
  input instead of only ever reading `inputs.first()`.

### 3. Regression test uses a real Candle fixture with distinct prompts, not the mock
`scheduler_batches_concurrent_requests_for_same_alias` (mock model, identical
prompts) is left as-is — it still validates scheduler metrics/batching
behavior, a different concern. A new test,
`scheduler_routes_distinct_concurrent_prompts_to_their_own_response`, loads
the existing tiny Llama fixture (`write_tachyon_tiny_fixture`, 4-word vocab:
`<unk> hello tachyon mesh`), computes reference outputs for three distinct
prompts sequentially, then re-issues all three concurrently through a
`Barrier` (same synchronization pattern as the existing scheduler test) and
asserts each caller's output matches its own prompt's reference — not
another prompt's. Verified this test fails (reproducing the original bug
symptom) when `process_batch`'s success arm is reverted to
`outputs.into_iter().next()` broadcast-to-all, and passes with the fix.

## Risks / Trade-offs

- **[Trade-off] No throughput gain from this change alone.** A batch of N
  requests still costs N sequential forward-pass-chains, same as before —
  this only fixes *which* output goes to *which* caller. Explicitly scoped
  as a Non-Goal; flagged as a follow-up change once paged attention's
  batch-shaped KV helpers are proven out further (this series has
  consistently deferred GPU-batched execution work behind a real-hardware
  validation cycle, and that cycle hasn't been designed yet for N>1
  sequences in one forward pass).
- **[Risk] Whole-batch error semantics unchanged, not per-prompt.** If one
  prompt in a batch is malformed (e.g. produces zero tokens), the entire
  batch's response is that error, not just that one job's. This matches
  today's existing behavior for whole-batch failures and is not made worse
  by this change; per-prompt error isolation is a smaller, independent
  future improvement if it turns out to matter in practice.
