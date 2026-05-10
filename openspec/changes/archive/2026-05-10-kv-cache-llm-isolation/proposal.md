# Proposal: KV-Cache LLM Isolation

## Problem

The `kv-cache` topology node type existed as a visual declaration with no backend
implementation. There was no storage table, no model-aware namespacing, and
critically no guard preventing a node from accepting inference cache writes for
an LLM it does not host. In a multi-node mesh, this would cause:

- **Stale cache pollution** — a node that hosts `mistral-7b` would silently
  accept writes for `llama-3` entries that can never be served correctly.
- **Cross-model key collisions** — without per-model key namespacing, two
  different models could collide on the same cache key.
- **Meaningless replication** — the CDC/gossip path would replicate KV states
  to every node regardless of whether the target node loads the referenced LLM.

## What Changes

### Storage (`core-host/src/store/mod.rs`)

New `KV_CACHE_TABLE` (ReDB `TableDefinition<&str, &[u8]>`) with key format:

```
{model_ref}/{tenant}/{cache_key}
```

The `model_ref` prefix physically partitions entries by LLM. A prefix scan on
`{model_ref}/` is sufficient to evict all entries for a single model without
touching any other model's state.

New `CoreStore` methods:
- `kv_cache_get(model, tenant, key)` — lazy TTL eviction on read
- `kv_cache_put(model, tenant, key, value, ttl)` — stores a `KvCacheEntry` with optional expiry
- `kv_cache_delete(model, tenant, key)`
- `kv_cache_evict_model(model)` — prefix-scan eviction, returns count
- `kv_cache_stats(model)` — live entry count, byte total, expired count

### Config (`core-host/src/host_core/domain_types.rs`)

New `IntegrityKvCacheConfig` struct in `IntegrityConfig.kv_caches`:
- `name` — logical cache identifier
- `model_ref` — LLM alias this cache is bound to (no slashes allowed)
- `max_ttl_seconds` — optional expiry
- `eviction_policy` — LRU / LFU / FIFO (default LRU)
- `tenant_isolation` — isolate per `x-tachyon-tenant` header (default `true`)

### Handlers (`core-host/src/host_core/kv_cache.rs`)

| Method | Path | Description |
|---|---|---|
| `GET` | `/api/kv-cache/{model}/{key}` | Read a cache entry (404 on miss/expiry) |
| `PUT` | `/api/kv-cache/{model}/{key}` | Write — **503 if model not hot** |
| `DELETE` | `/api/kv-cache/{model}/{key}` | Delete a single entry |
| `DELETE` | `/admin/kv-cache/{model}` | Evict all entries for a model |
| `GET` | `/admin/kv-cache/{model}/stats` | Entry count + byte usage |

**Model-hot guard on writes**: `PUT /api/kv-cache/{model}/*` returns
`503 Service Unavailable` when `ai_runtime.loaded_model_aliases()` does not
include `model_ref`. This is the core invariant — only the node running the LLM
can write its inference state.

### Validation (`core-host/src/host_core/integrity_config.rs`)

`validate_kv_caches()` enforces:
- Non-empty `name` and `model_ref`
- No slashes in `model_ref`
- Unique `name` values across all declared caches

## Caveats

**CDC/gossip replication filter**: this change does not yet filter kv-cache
entries from the replication path. A node that evicts a model's cache will still
receive replicated entries from a peer that hosts that model. A follow-up change
should extend `system-faas-cdc` to skip replication of `kv_cache` entries for
models not present in `hot_model_aliases()` on the destination node.
