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
//! `candle_llm_runtime.rs` wires this in for Llama on CUDA
//! (`wire-paged-attention-decode-path`'s tasks.md Section 3).

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use candle_core::{Device, Result as CandleResult, Tensor};
use std::{
    collections::HashMap,
    fmt, fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::Instant,
};

use crate::{RouteQos, SchedulerConfig, SchedulerSpillTier, SchedulerTierPreemptible};

/// A physical KV block's id within a [`PagedBlockPool`]. Meaningful only
/// relative to the pool that issued it.
pub(crate) type BlockId = u32;
const AES_GCM_TAG_BYTES: u64 = 16;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct KvSpillBlockKey {
    tenant_id: String,
    sequence_id: String,
    logical_block_index: u32,
}

impl KvSpillBlockKey {
    pub(crate) fn new(
        tenant_id: impl Into<String>,
        sequence_id: impl Into<String>,
        logical_block_index: u32,
    ) -> Result<Self, PagedKvError> {
        let tenant_id = tenant_id.into();
        let sequence_id = sequence_id.into();
        validate_scheduler_compatible_tenant_id(&tenant_id)?;
        validate_spill_key_segment(&sequence_id)?;
        Ok(Self {
            tenant_id,
            sequence_id,
            logical_block_index,
        })
    }

    fn path_stem(&self) -> String {
        format!(
            "kv-spill-s-{}-b-{}",
            hex::encode(self.sequence_id.as_bytes()),
            self.logical_block_index
        )
    }
}

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
    /// A non-preemptible QoS tier attempted to spill a KV block.
    NonPreemptibleTier(RouteQos),
    /// Neither pinned RAM nor the configured NVMe overflow tier has capacity.
    SpillCapacityExhausted {
        requested_bytes: u64,
        pinned_ram_available_bytes: u64,
        nvme_available_bytes: u64,
    },
    /// Tenant id or spill root would escape the per-tenant spill boundary.
    InvalidTenantId(String),
    /// A logical sequence/block key is already present in a spill tier.
    DuplicateSpillRecord(String),
    /// Spill encryption or authentication failed.
    Crypto(String),
    /// Raw spill block I/O failed.
    SpillIo(String),
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
            Self::NonPreemptibleTier(qos) => write!(
                f,
                "paged KV blocks for {qos:?} requests are not preemptible"
            ),
            Self::SpillCapacityExhausted {
                requested_bytes,
                pinned_ram_available_bytes,
                nvme_available_bytes,
            } => write!(
                f,
                "paged KV spill capacity exhausted: requested {requested_bytes} bytes, pinned RAM has {pinned_ram_available_bytes} bytes, NVMe has {nvme_available_bytes} bytes"
            ),
            Self::InvalidTenantId(tenant) => {
                write!(f, "tenant id `{tenant}` is not valid for KV spill isolation")
            }
            Self::DuplicateSpillRecord(key) => {
                write!(f, "paged KV spill record `{key}` already exists")
            }
            Self::Crypto(detail) => write!(f, "paged KV spill crypto failed: {detail}"),
            Self::SpillIo(detail) => write!(f, "paged KV spill I/O failed: {detail}"),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KvBlockTier {
    Vram,
    PinnedRam,
    Nvme,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KvPreemptionMode {
    Recompute,
    Swap,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct KvTieringMetricsSnapshot {
    pub(crate) vram_blocks: u64,
    pub(crate) pinned_ram_blocks: u64,
    pub(crate) nvme_blocks: u64,
    pub(crate) recompute_preemptions: u64,
    pub(crate) swap_preemptions: u64,
    pub(crate) spilled_bytes: u64,
    pub(crate) restored_bytes: u64,
    pub(crate) resume_latency_p50_us: u64,
    pub(crate) resume_latency_p99_us: u64,
}

#[derive(Debug, Default)]
struct KvTieringMetrics {
    recompute_preemptions: u64,
    swap_preemptions: u64,
    spilled_bytes: u64,
    restored_bytes: u64,
    resume_latencies_us: Vec<u64>,
}

impl KvTieringMetrics {
    fn snapshot(
        &self,
        records: &HashMap<KvSpillBlockKey, SpillRecord>,
    ) -> KvTieringMetricsSnapshot {
        let mut latencies = self.resume_latencies_us.clone();
        latencies.sort_unstable();
        KvTieringMetricsSnapshot {
            vram_blocks: records
                .values()
                .filter(|record| record.tier == KvBlockTier::Vram)
                .count() as u64,
            pinned_ram_blocks: records
                .values()
                .filter(|record| record.tier == KvBlockTier::PinnedRam)
                .count() as u64,
            nvme_blocks: records
                .values()
                .filter(|record| record.tier == KvBlockTier::Nvme)
                .count() as u64,
            recompute_preemptions: self.recompute_preemptions,
            swap_preemptions: self.swap_preemptions,
            spilled_bytes: self.spilled_bytes,
            restored_bytes: self.restored_bytes,
            resume_latency_p50_us: percentile(&latencies, 50),
            resume_latency_p99_us: percentile(&latencies, 99),
        }
    }
}

#[derive(Clone, Debug)]
struct SpillPointer {
    path: PathBuf,
    offset: u64,
    len: u64,
    nonce: [u8; 12],
}

#[derive(Clone, Debug)]
struct SpillRecord {
    tenant_id: String,
    tier: KvBlockTier,
    bytes: u64,
    pinned_ram: Option<Vec<u8>>,
    nvme: Option<SpillPointer>,
}

#[derive(Clone, Debug)]
pub(crate) struct KvBlockPayload {
    pub(crate) key: KvSpillBlockKey,
    pub(crate) tenant_id: String,
    pub(crate) block_id: BlockId,
    pub(crate) bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(crate) struct KvTierPagerConfig {
    pub(crate) pinned_ram_pool_bytes: u64,
    pub(crate) spill_budget_bytes: u64,
    pub(crate) spill_tier_max: SchedulerSpillTier,
    pub(crate) tier_preemptible: SchedulerTierPreemptible,
}

impl KvTierPagerConfig {
    pub(crate) fn from_scheduler(config: &SchedulerConfig) -> Self {
        Self {
            pinned_ram_pool_bytes: config.pinned_ram_pool_bytes,
            spill_budget_bytes: config.spill_budget_bytes,
            spill_tier_max: config.spill_tier_max,
            tier_preemptible: config.tier_preemptible,
        }
    }
}

/// Local paged-KV tier manager for evicting blocks VRAM -> pinned host RAM ->
/// encrypted tenant-isolated NVMe spill. It owns only host-side block payloads;
/// the CUDA runtime remains responsible for copying actual K/V tensors into and
/// out of these byte buffers at safe request boundaries.
#[derive(Debug)]
pub(crate) struct KvTierPager {
    config: KvTierPagerConfig,
    spill_root: PathBuf,
    tde_key: [u8; 32],
    pinned_ram_used_bytes: u64,
    nvme_used_bytes: u64,
    records: HashMap<KvSpillBlockKey, SpillRecord>,
    metrics: KvTieringMetrics,
}

impl KvTierPager {
    pub(crate) fn new(
        config: KvTierPagerConfig,
        spill_root: impl Into<PathBuf>,
        tde_key: [u8; 32],
    ) -> Self {
        Self {
            config,
            spill_root: spill_root.into(),
            tde_key,
            pinned_ram_used_bytes: 0,
            nvme_used_bytes: 0,
            records: HashMap::new(),
            metrics: KvTieringMetrics::default(),
        }
    }

    pub(crate) fn choose_preemption_mode(
        prefill_cost_us: u64,
        swap_bytes: u64,
        swap_bandwidth_bytes_per_us: u64,
    ) -> KvPreemptionMode {
        let bandwidth = swap_bandwidth_bytes_per_us.max(1);
        let swap_cost_us = swap_bytes.div_ceil(bandwidth);
        if prefill_cost_us <= swap_cost_us {
            KvPreemptionMode::Recompute
        } else {
            KvPreemptionMode::Swap
        }
    }

    pub(crate) fn preempt_block(
        &mut self,
        qos: RouteQos,
        payload: KvBlockPayload,
        prefill_cost_us: u64,
        swap_bandwidth_bytes_per_us: u64,
    ) -> Result<KvPreemptionMode, PagedKvError> {
        if !self.config.tier_preemptible.is_preemptible(qos) {
            return Err(PagedKvError::NonPreemptibleTier(qos));
        }
        validate_scheduler_compatible_tenant_id(&payload.tenant_id)?;
        if payload.key.tenant_id != payload.tenant_id {
            return Err(PagedKvError::InvalidTenantId(payload.tenant_id));
        }
        let mode = Self::choose_preemption_mode(
            prefill_cost_us,
            payload.bytes.len() as u64,
            swap_bandwidth_bytes_per_us,
        );
        match mode {
            KvPreemptionMode::Recompute => {
                self.metrics.recompute_preemptions += 1;
                Ok(mode)
            }
            KvPreemptionMode::Swap => {
                self.spill_block(payload)?;
                self.metrics.swap_preemptions += 1;
                Ok(mode)
            }
        }
    }

    pub(crate) fn restore_block(&mut self, key: &KvSpillBlockKey) -> Result<Vec<u8>, PagedKvError> {
        let started = Instant::now();
        let tier = self
            .records
            .get(key)
            .ok_or_else(|| {
                PagedKvError::SpillIo(format!("block {key:?} is not present in any spill tier"))
            })?
            .tier;
        let bytes = match tier {
            KvBlockTier::Vram => self
                .records
                .remove(key)
                .and_then(|record| record.pinned_ram)
                .unwrap_or_default(),
            KvBlockTier::PinnedRam => {
                let record = self.records.remove(key).ok_or_else(|| {
                    PagedKvError::SpillIo(format!("block {key:?} is not present in any spill tier"))
                })?;
                self.pinned_ram_used_bytes =
                    self.pinned_ram_used_bytes.saturating_sub(record.bytes);
                record.pinned_ram.unwrap_or_default()
            }
            KvBlockTier::Nvme => {
                let pointer = self
                    .records
                    .get(key)
                    .and_then(|record| record.nvme.clone())
                    .ok_or_else(|| {
                        PagedKvError::SpillIo(format!("block {key:?} is missing its spill pointer"))
                    })?;
                let bytes = read_encrypted_block(&pointer, &self.tde_key)?;
                self.reclaim_nvme_spill(pointer)?;
                self.records.remove(key);
                bytes
            }
        };
        self.metrics.restored_bytes += bytes.len() as u64;
        self.metrics
            .resume_latencies_us
            .push(started.elapsed().as_micros() as u64);
        Ok(bytes)
    }

    pub(crate) fn evict_block(&mut self, key: &KvSpillBlockKey) {
        if let Some(mut record) = self.records.remove(key) {
            if record.tier == KvBlockTier::PinnedRam {
                self.pinned_ram_used_bytes =
                    self.pinned_ram_used_bytes.saturating_sub(record.bytes);
            } else if record.tier == KvBlockTier::Nvme {
                if let Some(pointer) = record.nvme.take() {
                    let _ = self.reclaim_nvme_spill(pointer);
                }
            }
            if let Some(bytes) = record.pinned_ram.as_mut() {
                zeroize(bytes);
            }
        }
    }

    pub(crate) fn snapshot(&self) -> KvTieringMetricsSnapshot {
        self.metrics.snapshot(&self.records)
    }

    pub(crate) fn tenant_spill_path(&self, tenant_id: &str) -> Result<PathBuf, PagedKvError> {
        Ok(self.tenant_spill_dir(tenant_id)?.join("kv-spill.pool"))
    }

    fn spill_block(&mut self, payload: KvBlockPayload) -> Result<(), PagedKvError> {
        if self.records.contains_key(&payload.key) {
            return Err(PagedKvError::DuplicateSpillRecord(format!(
                "{:?}",
                payload.key
            )));
        }
        let requested = payload.bytes.len() as u64;
        let pinned_available = self
            .config
            .pinned_ram_pool_bytes
            .saturating_sub(self.pinned_ram_used_bytes);
        if requested <= pinned_available {
            self.pinned_ram_used_bytes += requested;
            self.metrics.spilled_bytes += requested;
            self.records.insert(
                payload.key,
                SpillRecord {
                    tenant_id: payload.tenant_id,
                    tier: KvBlockTier::PinnedRam,
                    bytes: requested,
                    pinned_ram: Some(payload.bytes),
                    nvme: None,
                },
            );
            return Ok(());
        }

        let physical_requested = encrypted_spill_len(requested)?;
        let nvme_available = self
            .config
            .spill_budget_bytes
            .saturating_sub(self.nvme_used_bytes);
        if self.config.spill_tier_max != SchedulerSpillTier::Nvme
            || physical_requested > nvme_available
        {
            return Err(PagedKvError::SpillCapacityExhausted {
                requested_bytes: physical_requested,
                pinned_ram_available_bytes: pinned_available,
                nvme_available_bytes: nvme_available,
            });
        }

        let nonce = random_spill_nonce();
        let path = self.block_spill_path(&payload.key, payload.block_id, nonce)?;
        let pointer = write_encrypted_block(&path, &payload.bytes, nonce, &self.tde_key)?;
        self.nvme_used_bytes += pointer.len;
        self.metrics.spilled_bytes += requested;
        self.records.insert(
            payload.key,
            SpillRecord {
                tenant_id: payload.tenant_id,
                tier: KvBlockTier::Nvme,
                bytes: pointer.len,
                pinned_ram: None,
                nvme: Some(pointer),
            },
        );
        Ok(())
    }

    fn tenant_spill_dir(&self, tenant_id: &str) -> Result<PathBuf, PagedKvError> {
        validate_scheduler_compatible_tenant_id(tenant_id)?;
        Ok(self.spill_root.join(tenant_path_segment(tenant_id)))
    }

    fn block_spill_path(
        &self,
        key: &KvSpillBlockKey,
        block_id: BlockId,
        nonce: [u8; 12],
    ) -> Result<PathBuf, PagedKvError> {
        Ok(self.tenant_spill_dir(&key.tenant_id)?.join(format!(
            "{}-p-{block_id}-n-{}.bin",
            key.path_stem(),
            hex::encode(nonce)
        )))
    }

    fn reclaim_nvme_spill(&mut self, pointer: SpillPointer) -> Result<(), PagedKvError> {
        fs::remove_file(&pointer.path).map_err(|error| {
            PagedKvError::SpillIo(format!(
                "failed to remove spill file `{}`: {error}",
                pointer.path.display()
            ))
        })?;
        self.nvme_used_bytes = self.nvme_used_bytes.saturating_sub(pointer.len);
        Ok(())
    }
}

fn write_encrypted_block(
    path: &Path,
    plaintext: &[u8],
    nonce: [u8; 12],
    key: &[u8; 32],
) -> Result<SpillPointer, PagedKvError> {
    let parent = path.parent().ok_or_else(|| {
        PagedKvError::SpillIo(format!("spill path `{}` has no parent", path.display()))
    })?;
    fs::create_dir_all(parent).map_err(|error| PagedKvError::SpillIo(error.to_string()))?;
    let nonce_ref = Nonce::try_from(nonce.as_slice())
        .map_err(|_| PagedKvError::Crypto("invalid nonce".to_owned()))?;
    let ciphertext = Aes256Gcm::new(key.into())
        .encrypt(&nonce_ref, plaintext)
        .map_err(|_| PagedKvError::Crypto("AES-GCM encryption failed".to_owned()))?;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|error| PagedKvError::SpillIo(error.to_string()))?;
    file.write_all(&ciphertext)
        .map_err(|error| PagedKvError::SpillIo(error.to_string()))?;
    file.sync_data()
        .map_err(|error| PagedKvError::SpillIo(error.to_string()))?;
    Ok(SpillPointer {
        path: path.to_path_buf(),
        offset: 0,
        len: ciphertext.len() as u64,
        nonce,
    })
}

fn read_encrypted_block(pointer: &SpillPointer, key: &[u8; 32]) -> Result<Vec<u8>, PagedKvError> {
    let mut file =
        fs::File::open(&pointer.path).map_err(|error| PagedKvError::SpillIo(error.to_string()))?;
    file.seek(SeekFrom::Start(pointer.offset))
        .map_err(|error| PagedKvError::SpillIo(error.to_string()))?;
    let mut ciphertext = vec![0_u8; pointer.len as usize];
    file.read_exact(&mut ciphertext)
        .map_err(|error| PagedKvError::SpillIo(error.to_string()))?;
    let nonce = Nonce::try_from(pointer.nonce.as_slice())
        .map_err(|_| PagedKvError::Crypto("invalid nonce".to_owned()))?;
    Aes256Gcm::new(key.into())
        .decrypt(&nonce, ciphertext.as_slice())
        .map_err(|_| PagedKvError::Crypto("AES-GCM authentication failed".to_owned()))
}

fn validate_scheduler_compatible_tenant_id(tenant_id: &str) -> Result<(), PagedKvError> {
    let valid = !tenant_id.is_empty()
        && tenant_id.trim() == tenant_id
        && !tenant_id.contains("..")
        && !tenant_id.contains('/')
        && !tenant_id.contains('\\')
        && tenant_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':' | '@'));
    if valid {
        Ok(())
    } else {
        Err(PagedKvError::InvalidTenantId(tenant_id.to_owned()))
    }
}

fn validate_spill_key_segment(segment: &str) -> Result<(), PagedKvError> {
    let valid = !segment.is_empty()
        && segment.trim() == segment
        && !segment.contains("..")
        && !segment.contains('/')
        && !segment.contains('\\');
    if valid {
        Ok(())
    } else {
        Err(PagedKvError::InvalidTenantId(segment.to_owned()))
    }
}

fn tenant_path_segment(tenant_id: &str) -> String {
    format!("t-{}", hex::encode(tenant_id.as_bytes()))
}

fn encrypted_spill_len(plaintext_len: u64) -> Result<u64, PagedKvError> {
    plaintext_len
        .checked_add(AES_GCM_TAG_BYTES)
        .ok_or_else(|| PagedKvError::SpillIo("spill block length overflowed".to_owned()))
}

fn random_spill_nonce() -> [u8; 12] {
    rand::random()
}

fn zeroize(bytes: &mut [u8]) {
    for byte in bytes {
        *byte = 0;
    }
}

fn percentile(sorted_values: &[u64], pct: usize) -> u64 {
    if sorted_values.is_empty() {
        return 0;
    }
    let index = ((sorted_values.len() - 1) * pct).div_ceil(100);
    sorted_values[index]
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

    fn kv_key(tenant: &str, sequence: &str, logical_block_index: u32) -> KvSpillBlockKey {
        KvSpillBlockKey::new(tenant, sequence, logical_block_index)
            .expect("test spill key should be valid")
    }

    fn kv_payload(
        tenant: &str,
        sequence: &str,
        logical_block_index: u32,
        physical_block_id: BlockId,
        bytes: impl Into<Vec<u8>>,
    ) -> KvBlockPayload {
        KvBlockPayload {
            key: kv_key(tenant, sequence, logical_block_index),
            tenant_id: tenant.to_owned(),
            block_id: physical_block_id,
            bytes: bytes.into(),
        }
    }

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

    #[test]
    fn preemption_mode_prefers_recompute_when_prefill_is_cheaper_than_swap() {
        assert_eq!(
            KvTierPager::choose_preemption_mode(10, 10_000, 100),
            KvPreemptionMode::Recompute
        );
        assert_eq!(
            KvTierPager::choose_preemption_mode(1_000, 10_000, 100),
            KvPreemptionMode::Swap
        );
    }

    #[test]
    fn realtime_blocks_are_not_preemptible() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut pager = KvTierPager::new(
            KvTierPagerConfig {
                pinned_ram_pool_bytes: 1024,
                spill_budget_bytes: 1024,
                spill_tier_max: SchedulerSpillTier::Nvme,
                tier_preemptible: SchedulerTierPreemptible::default(),
            },
            temp.path(),
            [7; 32],
        );

        let error = pager
            .preempt_block(
                RouteQos::RealTime,
                kv_payload("tenant-a", "seq-a", 0, 1, b"secret-kv"),
                1_000,
                1,
            )
            .expect_err("RealTime blocks must not be paged");
        assert!(matches!(
            error,
            PagedKvError::NonPreemptibleTier(RouteQos::RealTime)
        ));
    }

    #[test]
    fn pinned_ram_tier_round_trips_and_records_metrics() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut pager = KvTierPager::new(
            KvTierPagerConfig {
                pinned_ram_pool_bytes: 1024,
                spill_budget_bytes: 0,
                spill_tier_max: SchedulerSpillTier::Ram,
                tier_preemptible: SchedulerTierPreemptible::default(),
            },
            temp.path(),
            [9; 32],
        );

        let mode = pager
            .preempt_block(
                RouteQos::Standard,
                kv_payload("tenant-a", "seq-a", 0, 4, b"host-ram-kv"),
                100,
                1,
            )
            .expect("standard block should spill to pinned RAM");
        assert_eq!(mode, KvPreemptionMode::Swap);
        let snapshot = pager.snapshot();
        assert_eq!(snapshot.pinned_ram_blocks, 1);
        assert_eq!(snapshot.nvme_blocks, 0);
        assert_eq!(snapshot.swap_preemptions, 1);

        let restored = pager
            .restore_block(&kv_key("tenant-a", "seq-a", 0))
            .expect("block should restore");
        assert_eq!(restored, b"host-ram-kv");
        let snapshot = pager.snapshot();
        assert_eq!(snapshot.pinned_ram_blocks, 0);
        assert_eq!(snapshot.restored_bytes, b"host-ram-kv".len() as u64);
    }

    #[test]
    fn duplicate_pinned_spill_key_is_rejected_before_charging_capacity() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut pager = KvTierPager::new(
            KvTierPagerConfig {
                pinned_ram_pool_bytes: 32,
                spill_budget_bytes: 0,
                spill_tier_max: SchedulerSpillTier::Ram,
                tier_preemptible: SchedulerTierPreemptible::default(),
            },
            temp.path(),
            [11; 32],
        );

        pager
            .preempt_block(
                RouteQos::Standard,
                kv_payload("tenant-a", "seq-a", 0, 4, b"first-kv"),
                100,
                1,
            )
            .expect("first logical block should spill");
        let error = pager
            .preempt_block(
                RouteQos::Standard,
                kv_payload("tenant-a", "seq-a", 0, 5, b"replacement-kv"),
                100,
                1,
            )
            .expect_err("duplicate logical block should be rejected");
        assert!(matches!(error, PagedKvError::DuplicateSpillRecord(_)));

        let snapshot = pager.snapshot();
        assert_eq!(snapshot.pinned_ram_blocks, 1);
        assert_eq!(snapshot.spilled_bytes, b"first-kv".len() as u64);
        assert_eq!(
            pager
                .restore_block(&kv_key("tenant-a", "seq-a", 0))
                .expect("original block should remain restoreable"),
            b"first-kv"
        );
        assert_eq!(pager.snapshot().pinned_ram_blocks, 0);
    }

    #[test]
    fn nvme_tier_encrypts_and_isolates_tenant_spill_files() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut pager = KvTierPager::new(
            KvTierPagerConfig {
                pinned_ram_pool_bytes: 0,
                spill_budget_bytes: 4096,
                spill_tier_max: SchedulerSpillTier::Nvme,
                tier_preemptible: SchedulerTierPreemptible::default(),
            },
            temp.path(),
            [3; 32],
        );

        pager
            .preempt_block(
                RouteQos::Batch,
                kv_payload("tenant-a", "seq-a", 0, 10, b"tenant-a conversation KV"),
                1_000,
                1,
            )
            .expect("tenant-a block should spill to NVMe");
        pager
            .preempt_block(
                RouteQos::Batch,
                kv_payload("tenant-b", "seq-b", 0, 11, b"tenant-b conversation KV"),
                1_000,
                1,
            )
            .expect("tenant-b block should spill to NVMe");

        let tenant_a_path = pager.tenant_spill_path("tenant-a").expect("tenant-a path");
        let tenant_b_path = pager.tenant_spill_path("tenant-b").expect("tenant-b path");
        assert_ne!(tenant_a_path, tenant_b_path);

        let tenant_a_file = fs::read_dir(tenant_a_path.parent().expect("tenant-a dir"))
            .expect("tenant-a spill dir should exist")
            .map(|entry| entry.expect("dir entry").path())
            .find(|path| path.extension().is_some_and(|extension| extension == "bin"))
            .expect("tenant-a spill file should exist");
        let on_disk = fs::read(&tenant_a_file).expect("spill file should exist");
        assert!(
            !on_disk
                .windows(b"tenant-a conversation KV".len())
                .any(|window| window == b"tenant-a conversation KV"),
            "spill file must not contain plaintext KV"
        );
        assert_eq!(
            pager
                .restore_block(&kv_key("tenant-a", "seq-a", 0))
                .expect("tenant-a restore"),
            b"tenant-a conversation KV"
        );
        assert_eq!(
            pager
                .restore_block(&kv_key("tenant-b", "seq-b", 0))
                .expect("tenant-b restore"),
            b"tenant-b conversation KV"
        );
        assert!(
            !tenant_a_file.exists(),
            "restoring the block should reclaim the physical spill file"
        );
    }

    #[test]
    fn nvme_spill_nonces_do_not_repeat_after_pager_restart() {
        let temp = tempfile::tempdir().expect("temp dir");
        let config = KvTierPagerConfig {
            pinned_ram_pool_bytes: 0,
            spill_budget_bytes: 4096,
            spill_tier_max: SchedulerSpillTier::Nvme,
            tier_preemptible: SchedulerTierPreemptible::default(),
        };
        let tde_key = [14; 32];
        let first_key = kv_key("tenant-a", "seq-a", 0);
        let second_key = kv_key("tenant-a", "seq-b", 0);

        let mut first_pager = KvTierPager::new(config.clone(), temp.path().join("first"), tde_key);
        first_pager
            .preempt_block(
                RouteQos::Batch,
                kv_payload("tenant-a", "seq-a", 0, 7, b"first-nonce-kv"),
                1_000,
                1,
            )
            .expect("first pager should spill to NVMe");
        let first_nonce = first_pager
            .records
            .get(&first_key)
            .and_then(|record| record.nvme.as_ref())
            .map(|pointer| pointer.nonce)
            .expect("first spill should have a nonce");

        let mut restarted_pager = KvTierPager::new(config, temp.path().join("second"), tde_key);
        restarted_pager
            .preempt_block(
                RouteQos::Batch,
                kv_payload("tenant-a", "seq-b", 0, 7, b"second-nonce-kv"),
                1_000,
                1,
            )
            .expect("recreated pager should spill to NVMe");
        let second_nonce = restarted_pager
            .records
            .get(&second_key)
            .and_then(|record| record.nvme.as_ref())
            .map(|pointer| pointer.nonce)
            .expect("second spill should have a nonce");

        assert_ne!(
            first_nonce, second_nonce,
            "recreating a pager with the same TDE key must not restart AES-GCM nonces"
        );
    }

    #[test]
    fn duplicate_nvme_spill_key_is_rejected_without_orphaning_files() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut pager = KvTierPager::new(
            KvTierPagerConfig {
                pinned_ram_pool_bytes: 0,
                spill_budget_bytes: 128,
                spill_tier_max: SchedulerSpillTier::Nvme,
                tier_preemptible: SchedulerTierPreemptible::default(),
            },
            temp.path(),
            [12; 32],
        );

        pager
            .preempt_block(
                RouteQos::Batch,
                kv_payload("tenant-a", "seq-a", 0, 7, b"first-nvme-kv"),
                1_000,
                1,
            )
            .expect("first block should spill to NVMe");
        let error = pager
            .preempt_block(
                RouteQos::Batch,
                kv_payload("tenant-a", "seq-a", 0, 8, b"replacement-nvme-kv"),
                1_000,
                1,
            )
            .expect_err("duplicate logical block should be rejected");
        assert!(matches!(error, PagedKvError::DuplicateSpillRecord(_)));

        let tenant_dir = temp.path().join(tenant_path_segment("tenant-a"));
        let spill_files = fs::read_dir(&tenant_dir)
            .expect("tenant spill dir")
            .map(|entry| entry.expect("dir entry").path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "bin"))
            .collect::<Vec<_>>();
        assert_eq!(
            spill_files.len(),
            1,
            "duplicate retry must not orphan a file"
        );
        assert_eq!(
            pager
                .restore_block(&kv_key("tenant-a", "seq-a", 0))
                .expect("original block should remain restoreable"),
            b"first-nvme-kv"
        );
        assert!(
            fs::read_dir(tenant_dir)
                .expect("tenant spill dir")
                .all(|entry| entry.expect("dir entry").path().extension() != Some("bin".as_ref())),
            "restoring the original block should reclaim its only spill file"
        );
    }

    #[test]
    fn nvme_restore_keeps_record_when_read_fails() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut pager = KvTierPager::new(
            KvTierPagerConfig {
                pinned_ram_pool_bytes: 0,
                spill_budget_bytes: 128,
                spill_tier_max: SchedulerSpillTier::Nvme,
                tier_preemptible: SchedulerTierPreemptible::default(),
            },
            temp.path(),
            [13; 32],
        );

        pager
            .preempt_block(
                RouteQos::Batch,
                kv_payload("tenant-a", "seq-a", 0, 7, b"retryable-nvme-kv"),
                1_000,
                1,
            )
            .expect("block should spill to NVMe");

        let tenant_dir = temp.path().join(tenant_path_segment("tenant-a"));
        let spill_file = fs::read_dir(&tenant_dir)
            .expect("tenant spill dir")
            .map(|entry| entry.expect("dir entry").path())
            .find(|path| path.extension().is_some_and(|extension| extension == "bin"))
            .expect("spill file should exist");
        let hidden_file = spill_file.with_extension("missing");
        fs::rename(&spill_file, &hidden_file).expect("spill file should be movable");

        let first_error = pager
            .restore_block(&kv_key("tenant-a", "seq-a", 0))
            .expect_err("missing spill file should fail restore");
        assert!(
            first_error.to_string().contains("I/O failed"),
            "unexpected error: {first_error}"
        );
        assert_eq!(
            pager.snapshot().nvme_blocks,
            1,
            "failed restore must keep the spill record retryable"
        );

        fs::rename(&hidden_file, &spill_file).expect("spill file should be restorable");
        assert_eq!(
            pager
                .restore_block(&kv_key("tenant-a", "seq-a", 0))
                .expect("retry should restore once the file is back"),
            b"retryable-nvme-kv"
        );
        assert_eq!(pager.snapshot().nvme_blocks, 0);
        assert!(
            !spill_file.exists(),
            "successful retry should reclaim the physical spill file"
        );
    }

    #[test]
    fn configured_tenant_ids_are_encoded_for_spill_paths() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut pager = KvTierPager::new(
            KvTierPagerConfig {
                pinned_ram_pool_bytes: 0,
                spill_budget_bytes: 4096,
                spill_tier_max: SchedulerSpillTier::Nvme,
                tier_preemptible: SchedulerTierPreemptible::default(),
            },
            temp.path(),
            [6; 32],
        );

        pager
            .preempt_block(
                RouteQos::Standard,
                kv_payload(
                    "team.a:user@example",
                    "seq@example",
                    0,
                    12,
                    b"encoded tenant KV",
                ),
                1_000,
                1,
            )
            .expect("scheduler-compatible tenant id should spill");

        let spill_path = pager
            .tenant_spill_path("team.a:user@example")
            .expect("tenant path should be derivable");
        let tenant_dir = spill_path.parent().expect("tenant spill dir");
        assert!(tenant_dir.exists());
        assert_eq!(
            tenant_dir.file_name().and_then(|name| name.to_str()),
            Some("t-7465616d2e613a75736572406578616d706c65")
        );
        assert_eq!(
            pager
                .restore_block(&kv_key("team.a:user@example", "seq@example", 0))
                .expect("block should restore"),
            b"encoded tenant KV"
        );
    }

    #[test]
    fn nvme_restore_and_evict_reclaim_physical_spill_budget() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut pager = KvTierPager::new(
            KvTierPagerConfig {
                pinned_ram_pool_bytes: 0,
                spill_budget_bytes: 64,
                spill_tier_max: SchedulerSpillTier::Nvme,
                tier_preemptible: SchedulerTierPreemptible::default(),
            },
            temp.path(),
            [8; 32],
        );

        pager
            .preempt_block(
                RouteQos::Batch,
                kv_payload("tenant-a", "seq-a", 0, 20, vec![1; 16]),
                1_000,
                1,
            )
            .expect("first block should spill");
        let first_spill = fs::read_dir(temp.path().join(tenant_path_segment("tenant-a")))
            .expect("tenant spill dir")
            .map(|entry| entry.expect("dir entry").path())
            .find(|path| path.extension().is_some_and(|extension| extension == "bin"))
            .expect("first spill file");
        assert_eq!(fs::metadata(&first_spill).expect("metadata").len(), 32);

        assert_eq!(
            pager
                .restore_block(&kv_key("tenant-a", "seq-a", 0))
                .expect("first block should restore"),
            vec![1; 16]
        );
        assert!(!first_spill.exists());

        pager
            .preempt_block(
                RouteQos::Batch,
                kv_payload("tenant-a", "seq-a", 1, 21, vec![2; 16]),
                1_000,
                1,
            )
            .expect("restored budget should be reusable");
        pager.evict_block(&kv_key("tenant-a", "seq-a", 1));
        assert!(
            fs::read_dir(temp.path().join(tenant_path_segment("tenant-a")))
                .expect("tenant spill dir")
                .all(|entry| entry.expect("dir entry").path().extension() != Some("bin".as_ref())),
            "evicting an NVMe block should reclaim its physical spill file"
        );
    }

    #[test]
    fn logical_spill_keys_survive_physical_block_reuse() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut pager = KvTierPager::new(
            KvTierPagerConfig {
                pinned_ram_pool_bytes: 0,
                spill_budget_bytes: 128,
                spill_tier_max: SchedulerSpillTier::Nvme,
                tier_preemptible: SchedulerTierPreemptible::default(),
            },
            temp.path(),
            [10; 32],
        );

        pager
            .preempt_block(
                RouteQos::Standard,
                kv_payload("tenant-a", "seq-a", 0, 7, b"seq-a physical-slot-7"),
                1_000,
                1,
            )
            .expect("first logical block should spill");
        pager
            .preempt_block(
                RouteQos::Standard,
                kv_payload("tenant-b", "seq-b", 0, 7, b"seq-b reused-slot-7"),
                1_000,
                1,
            )
            .expect("second logical block should spill despite physical id reuse");

        assert_eq!(
            pager
                .restore_block(&kv_key("tenant-a", "seq-a", 0))
                .expect("first logical block should restore"),
            b"seq-a physical-slot-7"
        );
        assert_eq!(
            pager
                .restore_block(&kv_key("tenant-b", "seq-b", 0))
                .expect("second logical block should restore"),
            b"seq-b reused-slot-7"
        );
    }

    #[test]
    fn spill_capacity_exhaustion_is_reported_as_chaos_safe_cache_miss() {
        let temp = tempfile::tempdir().expect("temp dir");
        let mut pager = KvTierPager::new(
            KvTierPagerConfig {
                pinned_ram_pool_bytes: 4,
                spill_budget_bytes: 4,
                spill_tier_max: SchedulerSpillTier::Nvme,
                tier_preemptible: SchedulerTierPreemptible::default(),
            },
            temp.path(),
            [5; 32],
        );

        let error = pager
            .preempt_block(
                RouteQos::Standard,
                kv_payload("tenant-a", "seq-a", 0, 42, b"too-large-for-spill"),
                1_000,
                1,
            )
            .expect_err("full RAM/NVMe should fail closed");
        assert!(matches!(error, PagedKvError::SpillCapacityExhausted { .. }));
        assert_eq!(pager.snapshot().spilled_bytes, 0);
    }

    #[test]
    fn five_and_ten_agent_paging_scenarios_record_spill_and_resume_metrics() {
        let temp = tempfile::tempdir().expect("temp dir");
        let no_paging = KvTierPager::new(
            KvTierPagerConfig {
                pinned_ram_pool_bytes: 0,
                spill_budget_bytes: 0,
                spill_tier_max: SchedulerSpillTier::Ram,
                tier_preemptible: SchedulerTierPreemptible::default(),
            },
            temp.path().join("off"),
            [2; 32],
        );
        assert_eq!(no_paging.snapshot().spilled_bytes, 0);
        assert_eq!(no_paging.snapshot().resume_latency_p99_us, 0);

        let mut pager = KvTierPager::new(
            KvTierPagerConfig {
                pinned_ram_pool_bytes: 5 * 16,
                spill_budget_bytes: 10 * 16,
                spill_tier_max: SchedulerSpillTier::Nvme,
                tier_preemptible: SchedulerTierPreemptible::default(),
            },
            temp.path().join("on"),
            [4; 32],
        );

        for agent in 0..10 {
            let tenant = format!("tenant-{agent}");
            let payload = vec![agent as u8; 16];
            let mode = pager
                .preempt_block(
                    RouteQos::Standard,
                    kv_payload(&tenant, &format!("seq-{agent}"), 0, agent, payload),
                    1_000,
                    1,
                )
                .expect("agent KV block should spill");
            assert_eq!(mode, KvPreemptionMode::Swap);
        }

        let snapshot = pager.snapshot();
        assert_eq!(snapshot.pinned_ram_blocks, 5);
        assert_eq!(snapshot.nvme_blocks, 5);
        assert_eq!(snapshot.swap_preemptions, 10);
        assert_eq!(snapshot.spilled_bytes, 10 * 16);

        for agent in 0..10 {
            let restored = pager
                .restore_block(&kv_key(
                    &format!("tenant-{agent}"),
                    &format!("seq-{agent}"),
                    0,
                ))
                .expect("agent KV block should restore");
            assert_eq!(restored, vec![agent as u8; 16]);
        }

        let snapshot = pager.snapshot();
        assert_eq!(snapshot.pinned_ram_blocks, 0);
        assert_eq!(snapshot.nvme_blocks, 0);
        assert_eq!(snapshot.restored_bytes, 10 * 16);
        assert!(
            snapshot.resume_latency_p50_us <= snapshot.resume_latency_p99_us,
            "resume latency percentiles should be monotonic"
        );
    }

    #[test]
    fn tenant_ids_cannot_escape_spill_root() {
        let temp = tempfile::tempdir().expect("temp dir");
        let pager = KvTierPager::new(
            KvTierPagerConfig::from_scheduler(&SchedulerConfig::default()),
            temp.path(),
            [1; 32],
        );
        assert!(matches!(
            pager.tenant_spill_path("../tenant-b"),
            Err(PagedKvError::InvalidTenantId(_))
        ));
    }
}
