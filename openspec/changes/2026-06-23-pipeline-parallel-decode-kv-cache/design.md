# Design: Per-Stage KV Cache for Pipeline-Parallel Decode

## 1. Current state (what exists today)
```rust
// pipeline_parallel_llama.rs:38-45
pub(crate) struct PipelineStage {
    cfg: Config,
    layer_range: (u32, u32),
    wte: Option<Embedding>,
    blocks: Vec<TensorParallelBlock>,
    head: Option<(RmsNorm, Linear)>,
    device: Device,
}

// pipeline_parallel_llama.rs:116-126
fn run_stage(&self, layer_range: (u32, u32), input: &Tensor) -> CandleResult<Tensor> {
    debug_assert_eq!(layer_range, self.layer_range);
    let mut x = ...;
    let mut cache = TensorParallelCache::new(false, dtype, &self.cfg, &self.device)?; // fresh every call
    for (offset, block) in self.blocks.iter().enumerate() {
        let block_idx = self.layer_range.0 as usize + offset;
        x = block.forward(&x, 0, block_idx, &mut cache)?; // index_pos always 0
    }
    ...
}
```
`PipelineParallelLlama::forward` (line 205) calls `run_stage` once per stage, once, for one `tokens` tensor — there is no loop, no `index_pos`, and no persisted cache anywhere. Contrast with the working reference, `TensorParallelBlock::forward`'s caller in `tensor_parallel_llama.rs`, which already accepts `index_pos` and a long-lived `&mut TensorParallelCache` across multiple calls (proven by `tensor_parallel_llama_decodes_a_second_token_with_kv_cache`).

## 2. Target state

### 2.1 `PipelineStage` owns its cache
```rust
pub(crate) struct PipelineStage {
    cfg: Config,
    layer_range: (u32, u32),
    wte: Option<Embedding>,
    blocks: Vec<TensorParallelBlock>,
    head: Option<(RmsNorm, Linear)>,
    device: Device,
    cache: TensorParallelCache, // NEW: built once at `load()`, mutated across calls
}
```
`TensorParallelCache::new(true, dtype, cfg, device)` is called once in `PipelineStage::load`, with `use_kv_cache: true` (the existing `false` argument flips to `true`, matching how `TensorParallelLlama`'s decode-capable construction already works). `run_stage` becomes `&mut self` and takes `index_pos: usize` (mirroring `TensorParallelBlock::forward`'s own signature exactly):
```rust
fn run_stage(&mut self, index_pos: usize, input: &Tensor) -> CandleResult<Tensor> {
    let mut x = ...;
    for (offset, block) in self.blocks.iter().enumerate() {
        let block_idx = self.layer_range.0 as usize + offset;
        x = block.forward(&x, index_pos, block_idx, &mut self.cache)?;
    }
    ...
}
```
The `layer_range` sanity-check parameter that `run_stage` previously took (purely an assertion, never load-bearing per the existing doc comment) is dropped since the stage already knows its own range; callers no longer need to pass it back.

### 2.2 `PipelineStageExecutor` trait update
```rust
// parallel.rs
pub(crate) trait PipelineStageExecutor {
    fn run_stage(&mut self, index_pos: usize, input: &Tensor) -> CandleResult<Tensor>;
}
```
Changing `&self` to `&mut self` is required because the cache mutates; this is a mechanical, fully-contained change since the trait's only production implementor is `PipelineStage` and its only other implementor is the test-only `ClosureStageExecutor` in `parallel.rs`'s test module. `run_pipeline`/`run_pipeline_microbatched`'s call sites take `&mut [Box<dyn PipelineStageExecutor>]` (or equivalent) instead of `&[..]`.

### 2.3 `PipelineParallelLlama` decode loop
```rust
impl PipelineParallelLlama {
    /// Prefill: same as today, but now also primes each stage's persistent cache.
    pub(crate) fn forward_prefill(&mut self, tokens: &Tensor, transports: &[Box<dyn StageTransport>]) -> CandleResult<Tensor> {
        self.forward_at(0, tokens, transports)
    }

    /// Decode: one step at `index_pos`, `tokens` is `[batch, 1]`.
    pub(crate) fn forward_decode(&mut self, index_pos: usize, tokens: &Tensor, transports: &[Box<dyn StageTransport>]) -> CandleResult<Tensor> {
        self.forward_at(index_pos, tokens, transports)
    }

    fn forward_at(&mut self, index_pos: usize, tokens: &Tensor, transports: &[Box<dyn StageTransport>]) -> CandleResult<Tensor> {
        let mut activation = tokens.clone();
        for (i, stage) in self.stages.iter_mut().enumerate() {
            activation = stage.run_stage(index_pos, &activation)?;
            if let Some(transport) = transports.get(i) {
                activation = transport.send(activation)?;
            }
        }
        Ok(activation)
    }
}
```
This mirrors exactly how `ParallelModel::Tensor`'s caller in `candle_llm_runtime.rs` already separates prefill (`index_pos = 0`, full prompt) from decode (`index_pos = prompt_len + step`, single new token) — no new pattern is introduced, the existing one is replicated onto the pipeline path.

### 2.4 `candle_llm_runtime.rs` dispatch
The existing `ParallelModel::Pipeline { .. } => Err(self.execution_error(...))` arm is replaced with a decode loop structured identically to the `ParallelModel::Tensor` arm immediately preceding it in the same `match`: call `forward_prefill` once, sample/emit a token, then loop calling `forward_decode(index_pos, ..)` with `index_pos` incremented each iteration until the stop condition (max tokens / EOS) is reached — the same loop structure, sampling logic, and stop-condition handling already used for `ParallelModel::Tensor`, just calling into `PipelineParallelLlama` instead of `TensorParallelLlama`.

## 3. `StageTransport` lifetime question
`TcpStageTransport::send` (per `parallel.rs`) currently connects, sends one activation, blocks for the reply, and implicitly the connection's fate is left to the transport's own `send`/`serve_one` implementation. With decode now calling `forward_decode` once per generated token, a naive transport would reconnect per token — correct but with avoidable per-token TCP handshake overhead across a real cross-node deployment.

This change keeps `StageTransport`'s `send(&self, Tensor) -> CandleResult<Tensor>` contract unchanged for stage 0's prefill call, and addresses the per-token overhead by having the *implementation* (not the trait) retain a persistent connection internally (e.g., `TcpStageTransport` holding a `Mutex<Option<TcpStream>>` it lazily connects once and reuses for the lifetime of one generation request), so no trait-level or call-site change is required — `forward_decode`'s loop calls `transport.send(activation)` exactly as `forward_prefill` already does. If profiling or the per-call-site review during implementation finds this insufficient (e.g. the peer-side `serve_one` would also need to loop rather than serve one call and return), `StageTransport` gains an explicit `send_decode_step` distinct from `send`, but the default design keeps the trait surface unchanged.

## 4. Out of scope for this change
- Real wall-clock multi-process/multi-thread stage execution (`run_pipeline_microbatched`'s scheduler is unaffected).
- Cache eviction/paging for very long contexts — `TensorParallelCache`'s existing capacity behavior (whatever `TensorParallelLlama` already relies on) is reused unchanged.
