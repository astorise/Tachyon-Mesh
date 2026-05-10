# Tasks

## Storage
- [x] Add `KV_CACHE_TABLE` to `CoreStore` with key format `{model_ref}/{tenant}/{cache_key}`
- [x] Add `KvCacheEntry` (value + expires_at) and `KvCacheStats` structs
- [x] Implement `kv_cache_get` with lazy TTL eviction on read
- [x] Implement `kv_cache_put` with optional TTL
- [x] Implement `kv_cache_delete`
- [x] Implement `kv_cache_evict_model` (prefix-scan, returns eviction count)
- [x] Implement `kv_cache_stats` (live count, bytes, expired count)
- [x] Register `KV_CACHE_TABLE` in `initialize_tables`

## Config
- [x] Add `KvCacheEvictionPolicy` enum (LRU / LFU / FIFO)
- [x] Add `IntegrityKvCacheConfig` with name, model_ref, ttl, eviction_policy, tenant_isolation
- [x] Add `kv_caches: Vec<IntegrityKvCacheConfig>` to `IntegrityConfig`
- [x] Update `IntegrityConfig::default()` with `kv_caches: Vec::new()`

## Handlers
- [x] Create `host_core/kv_cache.rs` module
- [x] `kv_cache_get_handler` — 404 on unconfigured model or cache miss
- [x] `kv_cache_put_handler` — **503 model-hot guard**, 404 on unconfigured model
- [x] `kv_cache_delete_handler`
- [x] `kv_cache_evict_handler` (admin)
- [x] `kv_cache_stats_handler` (admin)
- [x] Register module in `host_core.rs`
- [x] Register routes in `app_runtime.rs`

## Validation
- [x] `validate_kv_caches` — empty name, empty model_ref, slash in model_ref, duplicate names
- [x] Call `validate_kv_caches` from `validate_integrity_config`

## Tests (17 passing)
- [x] `kv_cache_put_and_get_round_trips`
- [x] `kv_cache_returns_none_for_missing_key`
- [x] `kv_cache_entries_are_isolated_by_model` ← core isolation invariant
- [x] `kv_cache_entries_are_isolated_by_tenant`
- [x] `kv_cache_delete_removes_entry`
- [x] `kv_cache_evict_model_removes_only_target_model` ← cross-model safety
- [x] `kv_cache_expired_entry_returns_none_on_read`
- [x] `kv_cache_stats_counts_live_entries_per_model`
- [x] `kv_cache_get_returns_404_for_unconfigured_model`
- [x] `kv_cache_put_returns_404_for_unconfigured_model`
- [x] `kv_cache_put_returns_503_when_model_not_hot` ← guard invariant
- [x] `kv_cache_evict_returns_eviction_count`
- [x] `kv_cache_stats_returns_json_summary`
- [x] `validate_kv_caches_accepts_valid_config`
- [x] `validate_kv_caches_rejects_empty_name`
- [x] `validate_kv_caches_rejects_slash_in_model_ref`
- [x] `validate_kv_caches_rejects_duplicate_names`

## Caveat (follow-up)
- [ ] Extend CDC replication path to skip kv-cache entries for models not hot on the destination node
