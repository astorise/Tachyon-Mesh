//! Tachyon-owned block allocator and per-sequence block table for
//! `hardware_strategy.paged_attention`.
//!
//! This module is pure data-structure and tensor-layout logic: a fixed-size
//! pool of physical KV blocks, a free-list allocator, and a per-sequence
//! table mapping logical block index to physical block id. It has no
//! `candle-cuda` dependency and no knowledge of `CausalSelfAttention` or
//! `Llama` — it only builds the `block_table`/`seqlens_k` tensors that
//! `candle_transformers::models::llama::{Cache::set_paged_kv, PagedKvCache}`
//! (the seam added in `astorise/candle` tag `tachyon-v0.11.0-3`, see
//! `openspec/changes/wire-paged-attention-decode-path/design.md`) expects,
//! leaving allocation/eviction policy in `core-host` per that design's
//! Decision 1.
//!
//! Wiring this into the load/decode path (attaching a `PagedKvCache` via
//! `Cache::set_paged_kv` and growing/freeing a `SequenceBlockTable` per
//! request) is `wire-paged-attention-decode-path`'s tasks.md Section 3, not
//! yet done — nothing here is called from `candle_llm_runtime.rs` yet, hence
//! the module-wide `allow(dead_code)` below rather than `#[allow(dead_code)]`
//! disappearing item by item as Section 3 lands.
#![allow(dead_code)]

use candle_core::{Device, Result as CandleResult, Tensor};
use std::fmt;

/// A physical KV block's id within a [`PagedBlockPool`]. Meaningful only
/// relative to the pool that issued it.
pub(crate) type BlockId = u32;

#[derive(Debug)]
pub(crate) enum PagedKvError {
    /// [`PagedBlockPool::allocate_block`] was called with no free blocks left.
    PoolExhausted,
    /// [`PagedBlockPool::try_new_within_budget`] could not fit `min_blocks`
    /// blocks of `bytes_per_block` bytes each within `budget_bytes`.
    BudgetTooSmall {
        min_blocks: usize,
        bytes_per_block: u64,
        budget_bytes: u64,
    },
}

impl fmt::Display for PagedKvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PoolExhausted => write!(f, "paged KV block pool is exhausted"),
            Self::BudgetTooSmall {
                min_blocks,
                bytes_per_block,
                budget_bytes,
            } => write!(
                f,
                "paged KV cache budget of {budget_bytes} bytes cannot fit the minimum {min_blocks} blocks at {bytes_per_block} bytes/block"
            ),
        }
    }
}

impl std::error::Error for PagedKvError {}

/// Fixed-size pool of physical KV blocks with a free-list allocator. Owns no
/// tensors itself — the caller allocates the actual `key_cache`/`value_cache`
/// storage sized to `page_block_size * total_blocks` and only asks this pool
/// which physical block ids are free.
#[derive(Debug)]
pub(crate) struct PagedBlockPool {
    page_block_size: usize,
    total_blocks: usize,
    free_blocks: Vec<BlockId>,
}

impl PagedBlockPool {
    /// Creates a pool of `num_blocks` blocks, each holding `page_block_size`
    /// tokens, all initially free.
    pub(crate) fn new(page_block_size: usize, num_blocks: usize) -> Self {
        Self {
            page_block_size,
            total_blocks: num_blocks,
            free_blocks: (0..num_blocks as u32).rev().collect(),
        }
    }

    /// Sizes a pool from a byte budget instead of an explicit block count:
    /// as many blocks of `bytes_per_block` bytes as fit in `budget_bytes`,
    /// rejecting the budget outright if it can't fit at least `min_blocks`.
    /// `bytes_per_block` is the caller's responsibility to compute (typically
    /// `2 (K+V) * page_block_size * num_kv_heads * head_dim * dtype_size`).
    pub(crate) fn try_new_within_budget(
        page_block_size: usize,
        bytes_per_block: u64,
        budget_bytes: u64,
        min_blocks: usize,
    ) -> Result<Self, PagedKvError> {
        let max_blocks = budget_bytes
            .checked_div(bytes_per_block)
            .map_or(usize::MAX, |blocks| blocks as usize);
        if max_blocks < min_blocks {
            return Err(PagedKvError::BudgetTooSmall {
                min_blocks,
                bytes_per_block,
                budget_bytes,
            });
        }
        Ok(Self::new(page_block_size, max_blocks))
    }

    pub(crate) fn page_block_size(&self) -> usize {
        self.page_block_size
    }

    pub(crate) fn total_blocks(&self) -> usize {
        self.total_blocks
    }

    pub(crate) fn free_block_count(&self) -> usize {
        self.free_blocks.len()
    }

    /// Allocates one physical block, or [`PagedKvError::PoolExhausted`] if
    /// none are free.
    pub(crate) fn allocate_block(&mut self) -> Result<BlockId, PagedKvError> {
        self.free_blocks.pop().ok_or(PagedKvError::PoolExhausted)
    }

    /// Returns `blocks` to the free list. Does not validate that they were
    /// actually issued by this pool (or aren't already free) — callers are
    /// expected to free exactly the blocks a [`SequenceBlockTable`] holds,
    /// once, via [`SequenceBlockTable::free`].
    pub(crate) fn free_blocks(&mut self, blocks: impl IntoIterator<Item = BlockId>) {
        self.free_blocks.extend(blocks);
    }
}

/// A single sequence's logical-block-index → physical-block-id mapping.
/// Grows on demand as the sequence's token count crosses a `page_block_size`
/// boundary; the caller is responsible for calling [`Self::free`] when the
/// sequence completes or is evicted.
#[derive(Debug, Default)]
pub(crate) struct SequenceBlockTable {
    blocks: Vec<BlockId>,
}

impl SequenceBlockTable {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn blocks(&self) -> &[BlockId] {
        &self.blocks
    }

    /// Grows this table so it can hold `total_tokens` tokens, allocating
    /// whatever additional blocks are needed from `pool`. A no-op if the
    /// table already has enough blocks (e.g. called again for the same or a
    /// smaller token count).
    pub(crate) fn grow_to(
        &mut self,
        total_tokens: usize,
        pool: &mut PagedBlockPool,
    ) -> Result<(), PagedKvError> {
        let page_block_size = pool.page_block_size().max(1);
        let needed_blocks = total_tokens.div_ceil(page_block_size).max(1);
        while self.blocks.len() < needed_blocks {
            self.blocks.push(pool.allocate_block()?);
        }
        Ok(())
    }

    /// Returns every block this sequence holds to `pool` and clears the
    /// table, so it can be reused for a new sequence (or dropped).
    pub(crate) fn free(&mut self, pool: &mut PagedBlockPool) {
        pool.free_blocks(self.blocks.drain(..));
    }
}

/// Builds the `(batch_size, max_blocks)` physical block-id tensor that
/// `candle_transformers::models::llama::PagedKvCache::block_table` (and
/// `candle_flash_attn::flash_attn_varlen_paged_windowed`) expect: one row per
/// sequence in `tables`, right-padded with `0` up to the longest table's
/// block count. Padding entries are never read: `seqlens_k` bounds how much
/// of each row's paged history the kernel actually attends to.
pub(crate) fn build_block_table_tensor(
    tables: &[&SequenceBlockTable],
    device: &Device,
) -> CandleResult<Tensor> {
    let max_blocks = tables.iter().map(|t| t.blocks().len()).max().unwrap_or(0);
    let mut data = Vec::with_capacity(tables.len() * max_blocks);
    for table in tables {
        data.extend_from_slice(table.blocks());
        data.resize(data.len() + (max_blocks - table.blocks().len()), 0);
    }
    Tensor::from_vec(data, (tables.len(), max_blocks), device)
}

/// Builds the `(batch_size + 1,)` cumulative-length tensor
/// `flash_attn_varlen_paged_windowed`'s `seqlens_q`/`seqlens_k` expect:
/// `[0, lengths[0], lengths[0]+lengths[1], ...]`.
pub(crate) fn build_cumulative_seqlens_tensor(
    lengths: &[usize],
    device: &Device,
) -> CandleResult<Tensor> {
    let mut cumulative = Vec::with_capacity(lengths.len() + 1);
    let mut running_total = 0u32;
    cumulative.push(running_total);
    for &length in lengths {
        running_total += length as u32;
        cumulative.push(running_total);
    }
    Tensor::from_vec(cumulative, lengths.len() + 1, device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_allocate_and_free_through_the_pool() {
        let mut pool = PagedBlockPool::new(16, 4);
        assert_eq!(pool.free_block_count(), 4);

        let a = pool.allocate_block().expect("pool should have free blocks");
        let b = pool.allocate_block().expect("pool should have free blocks");
        assert_ne!(a, b, "distinct allocations must return distinct block ids");
        assert_eq!(pool.free_block_count(), 2);

        pool.free_blocks([a, b]);
        assert_eq!(
            pool.free_block_count(),
            4,
            "freed blocks must return to the pool"
        );
    }

    #[test]
    fn pool_exhaustion_is_a_typed_error_not_a_panic() {
        let mut pool = PagedBlockPool::new(16, 1);
        pool.allocate_block().expect("first allocation should fit");
        match pool.allocate_block() {
            Err(PagedKvError::PoolExhausted) => {}
            other => panic!("expected PoolExhausted, got {other:?}"),
        }
    }

    #[test]
    fn budget_sizing_rejects_a_budget_too_small_for_the_minimum() {
        let error = PagedBlockPool::try_new_within_budget(16, 1_000, 2_500, 4)
            .expect_err("2500 bytes cannot fit 4 blocks at 1000 bytes each");
        match error {
            PagedKvError::BudgetTooSmall {
                min_blocks,
                bytes_per_block,
                budget_bytes,
            } => {
                assert_eq!(min_blocks, 4);
                assert_eq!(bytes_per_block, 1_000);
                assert_eq!(budget_bytes, 2_500);
            }
            other => panic!("expected BudgetTooSmall, got {other:?}"),
        }
    }

    #[test]
    fn budget_sizing_fits_as_many_blocks_as_the_budget_allows() {
        let pool = PagedBlockPool::try_new_within_budget(16, 1_000, 4_500, 2)
            .expect("4500 bytes fits at least 2 blocks at 1000 bytes each");
        assert_eq!(pool.total_blocks(), 4);
        assert_eq!(pool.free_block_count(), 4);
    }

    #[test]
    fn sequence_table_grows_by_whole_blocks_and_is_idempotent() {
        let mut pool = PagedBlockPool::new(4, 8);
        let mut table = SequenceBlockTable::new();

        table
            .grow_to(1, &mut pool)
            .expect("growing to 1 token should allocate a single block");
        assert_eq!(table.blocks().len(), 1);

        table
            .grow_to(4, &mut pool)
            .expect("growing to exactly one block's worth stays at one block");
        assert_eq!(table.blocks().len(), 1);

        table
            .grow_to(5, &mut pool)
            .expect("crossing the block boundary allocates a second block");
        assert_eq!(table.blocks().len(), 2);

        // Idempotent: shrinking the requested token count never frees blocks.
        table
            .grow_to(1, &mut pool)
            .expect("growing to a smaller count is a no-op");
        assert_eq!(table.blocks().len(), 2);
    }

    #[test]
    fn sequence_table_growth_reports_pool_exhaustion() {
        let mut pool = PagedBlockPool::new(4, 1);
        let mut table = SequenceBlockTable::new();
        table
            .grow_to(4, &mut pool)
            .expect("the single available block should satisfy 4 tokens");
        match table.grow_to(5, &mut pool) {
            Err(PagedKvError::PoolExhausted) => {}
            other => panic!("expected PoolExhausted, got {other:?}"),
        }
    }

    #[test]
    fn freeing_a_sequence_table_returns_its_blocks_and_clears_it() {
        let mut pool = PagedBlockPool::new(4, 2);
        let mut table = SequenceBlockTable::new();
        table
            .grow_to(8, &mut pool)
            .expect("2 blocks should satisfy 8 tokens");
        assert_eq!(pool.free_block_count(), 0);

        table.free(&mut pool);
        assert_eq!(pool.free_block_count(), 2);
        assert!(table.blocks().is_empty());
    }

    #[test]
    fn block_table_tensor_right_pads_shorter_sequences_with_zero() {
        let mut pool = PagedBlockPool::new(4, 10);
        let mut long = SequenceBlockTable::new();
        long.grow_to(12, &mut pool)
            .expect("12 tokens need 3 blocks");
        let mut short = SequenceBlockTable::new();
        short.grow_to(4, &mut pool).expect("4 tokens need 1 block");

        let tensor =
            build_block_table_tensor(&[&long, &short], &Device::Cpu).expect("tensor should build");
        assert_eq!(tensor.dims(), &[2, 3]);
        let rows = tensor
            .to_vec2::<u32>()
            .expect("block table tensor should read back as u32");
        assert_eq!(rows[0], long.blocks());
        assert_eq!(rows[1][0], short.blocks()[0]);
        assert_eq!(rows[1][1], 0, "padding entry must be 0");
        assert_eq!(rows[1][2], 0, "padding entry must be 0");
    }

    #[test]
    fn block_table_tensor_is_empty_for_no_sequences() {
        let tensor = build_block_table_tensor(&[], &Device::Cpu).expect("tensor should build");
        assert_eq!(tensor.dims(), &[0, 0]);
    }

    #[test]
    fn cumulative_seqlens_tensor_matches_flash_attn_varlen_convention() {
        let tensor =
            build_cumulative_seqlens_tensor(&[3, 5, 2], &Device::Cpu).expect("tensor should build");
        assert_eq!(tensor.dims(), &[4]);
        let values = tensor
            .to_vec1::<u32>()
            .expect("seqlens tensor should read back as u32");
        assert_eq!(values, vec![0, 3, 8, 10]);
    }

    #[test]
    fn cumulative_seqlens_tensor_is_a_single_zero_for_no_sequences() {
        let tensor =
            build_cumulative_seqlens_tensor(&[], &Device::Cpu).expect("tensor should build");
        let values = tensor
            .to_vec1::<u32>()
            .expect("seqlens tensor should read back as u32");
        assert_eq!(values, vec![0]);
    }
}
