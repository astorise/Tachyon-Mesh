use anyhow::{Context, Result};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    collections::HashMap,
    fs,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

pub(crate) mod secrets;

const CWASM_CACHE_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("cwasm_cache");
/// LLM inference KV-cache entries. Key format: `{model_ref}/{tenant}/{cache_key}`.
/// The `model_ref` segment isolates entries per LLM so they are never served to
/// a node that doesn't host the corresponding model.
const KV_CACHE_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("kv_cache");
const METADATA_TABLE: TableDefinition<&str, &str> = TableDefinition::new("metadata");
const CWASM_ENGINE_HASH_KEY: &str = "cwasm_engine_hash";
const TLS_CERTS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("tls_certs");
const HIBERNATION_STATE_TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new("hibernation_state");
const DATA_EVENTS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("data_events");
/// Per-tenant HNSW vector index blobs. Key: `<tenant>/<index-name>`.
const VECTOR_INDICES_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("vector_indices");
/// Out-of-band AuthZ cache invalidation events. Producer: `system-faas-authz`.
/// Consumer: the `core-host` background subscriber that evicts the in-process
/// RBAC cache. Key: monotonic event id (zero-padded so range scans are ordered).
const AUTHZ_PURGE_OUTBOX_TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new("authz_purge_outbox");
/// Edge-to-cloud Change Data Capture outbox. Producer: the host's storage write
/// path for resources flagged `sync_to_cloud: true`. Consumer: `system-faas-cdc`.
/// Key: monotonic event id.
const DATA_MUTATION_OUTBOX_TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new("data_mutation_outbox");
/// Out-of-band fuel-metering events emitted post-execution. Producer: `core-host`.
/// Consumer: `system-faas-metering`. Key: monotonic event id.
const METERING_OUTBOX_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("metering_outbox");
/// Cluster-wide configuration update events emitted by `POST /admin/manifest`.
/// Producer: `core-host` after a signed manifest is accepted. Consumer: the
/// gossip-bridge which broadcasts `ConfigUpdateEvent` so peers can pull the
/// new manifest. Key: monotonic event id.
const CONFIG_UPDATE_OUTBOX_TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new("config_update_outbox");
const COMPRESSED_MAGIC_BYTES: &[&[u8]] = &[
    b"\xFF\xD8\xFF",
    b"\x89PNG",
    b"\x00\x00\x00\x18ftyp",
    b"PK\x03\x04",
    b"\x1F\x8B",
    b"\x28\xB5\x2F\xFD",
];

#[derive(Clone)]
pub(crate) struct CoreStore {
    db: Arc<Database>,
}

#[derive(Default)]
#[allow(dead_code)]
pub(crate) struct BaasQueryCache {
    entries: Mutex<HashMap<String, String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
pub(crate) struct VersionedRecord {
    pub(crate) schema_version: u32,
    pub(crate) payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct ConflictState {
    pub(crate) key: String,
    pub(crate) local_value: Vec<u8>,
    pub(crate) remote_value: Vec<u8>,
    pub(crate) local_clock: u64,
    pub(crate) remote_clock: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) struct ScanOptions {
    pub(crate) start_key: Vec<u8>,
    pub(crate) end_key: Vec<u8>,
    pub(crate) limit: usize,
    pub(crate) pushdown_filter_wasm: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum CoreStoreBucket {
    CwasmCache,
    TlsCerts,
    HibernationState,
    DataEvents,
    // Wired by follow-up changes (HNSW vector RAG / authz cache invalidation /
    // edge-to-cloud CDC sync). Schema lives here so the redb file is forward-
    // compatible — no migration is needed when those changes land.
    #[allow(dead_code)]
    VectorIndices,
    #[allow(dead_code)]
    AuthzPurgeOutbox,
    #[allow(dead_code)]
    DataMutationOutbox,
    MeteringOutbox,
    ConfigUpdateOutbox,
}

#[allow(dead_code)]
pub(crate) fn should_compress_blob(header: &[u8]) -> bool {
    !COMPRESSED_MAGIC_BYTES
        .iter()
        .any(|magic| header.starts_with(magic))
}

#[allow(dead_code)]
pub(crate) fn pipe_range_from_file(path: &Path, start: u64, end: u64) -> Result<Vec<u8>> {
    pipe_range_from_file_in_root(None, path, start, end)
}

#[allow(dead_code)]
pub(crate) fn pipe_range_from_file_in_root(
    root: Option<&Path>,
    path: &Path,
    start: u64,
    end: u64,
) -> Result<Vec<u8>> {
    if end < start {
        anyhow::bail!("invalid media range: end before start");
    }
    let canonical = fs::canonicalize(path)
        .with_context(|| format!("failed to canonicalize media path `{}`", path.display()))?;
    if let Some(root) = root {
        let canonical_root = fs::canonicalize(root)
            .with_context(|| format!("failed to canonicalize media root `{}`", root.display()))?;
        if !canonical.starts_with(&canonical_root) {
            anyhow::bail!(
                "media path `{}` escapes allowed root `{}`",
                canonical.display(),
                canonical_root.display()
            );
        }
    }
    let file = fs::File::open(&canonical)
        .with_context(|| format!("failed to open media blob `{}`", canonical.display()))?;
    let mmap = unsafe {
        memmap2::Mmap::map(&file)
            .with_context(|| format!("failed to mmap media blob `{}`", canonical.display()))?
    };
    let file_len = mmap.len() as u64;
    if start >= file_len {
        anyhow::bail!("media range start {start} exceeds file length {file_len}");
    }
    let end = end.min(file_len.saturating_sub(1));
    Ok(mmap[start as usize..=end as usize].to_vec())
}

#[allow(dead_code)]
pub(crate) fn transform_read_if_stale<F>(
    current_version: u32,
    record: VersionedRecord,
    shim: F,
) -> Result<Vec<u8>>
where
    F: FnOnce(u32, &[u8]) -> Result<Vec<u8>>,
{
    if record.schema_version < current_version {
        shim(record.schema_version, &record.payload)
    } else {
        Ok(record.payload)
    }
}

#[allow(dead_code)]
pub(crate) fn detect_split_brain(local_clock: u64, remote_clock: u64) -> bool {
    local_clock != remote_clock && local_clock != 0 && remote_clock != 0
}

#[allow(dead_code)]
pub(crate) fn resolve_conflict_with<F>(conflict: ConflictState, resolver: F) -> Result<Vec<u8>>
where
    F: FnOnce(&ConflictState) -> Result<Vec<u8>>,
{
    if detect_split_brain(conflict.local_clock, conflict.remote_clock) {
        resolver(&conflict)
    } else if conflict.remote_clock > conflict.local_clock {
        Ok(conflict.remote_value)
    } else {
        Ok(conflict.local_value)
    }
}

#[allow(dead_code)]
pub(crate) fn pushdown_wasmtime_config() -> wasmtime::Config {
    let mut config = wasmtime::Config::new();
    config.consume_fuel(true);
    config
}

#[allow(dead_code)]
pub(crate) fn execute_filtered_scan<I, F>(
    rows: I,
    options: &ScanOptions,
    mut evaluate: F,
) -> Result<Vec<(Vec<u8>, Vec<u8>)>>
where
    I: IntoIterator<Item = (Vec<u8>, Vec<u8>)>,
    F: FnMut(&[u8], &[u8], Option<&[u8]>) -> Result<bool>,
{
    let mut results = Vec::new();
    for (key, value) in rows {
        if !options.start_key.is_empty() && key < options.start_key {
            continue;
        }
        if !options.end_key.is_empty() && key >= options.end_key {
            continue;
        }
        if evaluate(&key, &value, options.pushdown_filter_wasm.as_deref())? {
            results.push((key, value));
            if results.len() >= options.limit {
                break;
            }
        }
    }
    Ok(results)
}

#[allow(dead_code)]
impl BaasQueryCache {
    pub(crate) fn get_or_insert_with<F>(
        &self,
        query: &str,
        datalog: &str,
        build: F,
    ) -> Result<String>
    where
        F: FnOnce() -> Result<String>,
    {
        let key = format!(
            "{}:{}",
            sha256_hex(query.as_bytes()),
            sha256_hex(datalog.as_bytes())
        );
        if let Some(cached) = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&key)
            .cloned()
        {
            return Ok(cached);
        }
        let sanitized = build()?;
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key, sanitized.clone());
        Ok(sanitized)
    }
}

#[allow(dead_code)]
fn sha256_hex(input: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(input))
}

#[derive(Debug, Serialize, Deserialize)]
struct DirectorySnapshot {
    entries: Vec<DirectoryEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DirectoryEntry {
    relative_path: String,
    kind: DirectoryEntryKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    body: Vec<u8>,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct VectorDocument {
    pub(crate) id: String,
    pub(crate) embedding: Vec<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) payload: Option<Vec<u8>>,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct VectorSearchMatch {
    pub(crate) id: String,
    pub(crate) score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) payload: Option<Vec<u8>>,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct VectorIndex {
    dim: usize,
    #[serde(default)]
    m: u32,
    #[serde(default)]
    ef_construction: u32,
    documents: Vec<VectorDocument>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum DirectoryEntryKind {
    Directory,
    File,
}

impl CoreStore {
    pub(crate) fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create core store parent directory `{}`",
                    parent.display()
                )
            })?;
        }

        let db = Database::create(path)
            .with_context(|| format!("failed to open embedded core store `{}`", path.display()))?;
        let store = Self { db: Arc::new(db) };
        store.initialize_tables()?;
        Ok(store)
    }

    fn initialize_tables(&self) -> Result<()> {
        let write_txn = self
            .db
            .begin_write()
            .context("failed to begin core store initialization transaction")?;
        {
            write_txn
                .open_table(METADATA_TABLE)
                .context("failed to open metadata table")?;
            write_txn
                .open_table(CWASM_CACHE_TABLE)
                .context("failed to open cwasm_cache table")?;
            write_txn
                .open_table(TLS_CERTS_TABLE)
                .context("failed to open tls_certs table")?;
            write_txn
                .open_table(HIBERNATION_STATE_TABLE)
                .context("failed to open hibernation_state table")?;
            write_txn
                .open_table(DATA_EVENTS_TABLE)
                .context("failed to open data_events table")?;
            write_txn
                .open_table(VECTOR_INDICES_TABLE)
                .context("failed to open vector_indices table")?;
            write_txn
                .open_table(AUTHZ_PURGE_OUTBOX_TABLE)
                .context("failed to open authz_purge_outbox table")?;
            write_txn
                .open_table(DATA_MUTATION_OUTBOX_TABLE)
                .context("failed to open data_mutation_outbox table")?;
            write_txn
                .open_table(METERING_OUTBOX_TABLE)
                .context("failed to open metering_outbox table")?;
            write_txn
                .open_table(CONFIG_UPDATE_OUTBOX_TABLE)
                .context("failed to open config_update_outbox table")?;
            write_txn
                .open_table(KV_CACHE_TABLE)
                .context("failed to open kv_cache table")?;
        }
        write_txn
            .commit()
            .context("failed to commit core store initialization transaction")
    }

    pub(crate) fn secure_cwasm_cache_bootstrap(&self, current_engine_hash: &str) -> Result<bool> {
        let write_txn = self
            .db
            .begin_write()
            .context("failed to begin cwasm cache bootstrap transaction")?;
        let mut purged = false;
        {
            let mut metadata = write_txn
                .open_table(METADATA_TABLE)
                .context("failed to open metadata table")?;
            let stored_hash = metadata
                .get(CWASM_ENGINE_HASH_KEY)
                .context("failed to read cwasm engine hash metadata")?
                .map(|value| value.value().to_owned());

            if let Some(stored_hash) = stored_hash {
                if stored_hash != current_engine_hash {
                    drop(metadata);
                    tracing::warn!(
                        "Purging stale Cwasm cache: Wasmtime engine compatibility hash changed"
                    );
                    let _ = write_txn.delete_table(CWASM_CACHE_TABLE);
                    write_txn
                        .open_table(CWASM_CACHE_TABLE)
                        .context("failed to recreate cwasm_cache table")?;
                    metadata = write_txn
                        .open_table(METADATA_TABLE)
                        .context("failed to reopen metadata table")?;
                    purged = true;
                }
            }

            metadata
                .insert(CWASM_ENGINE_HASH_KEY, current_engine_hash)
                .context("failed to persist cwasm engine hash metadata")?;
        }
        write_txn
            .commit()
            .context("failed to commit cwasm cache bootstrap transaction")?;
        Ok(purged)
    }

    pub(crate) fn get(&self, bucket: CoreStoreBucket, key: &str) -> Result<Option<Vec<u8>>> {
        let read_txn = self
            .db
            .begin_read()
            .context("failed to begin core store read transaction")?;
        match bucket {
            CoreStoreBucket::CwasmCache => read_bytes(&read_txn, CWASM_CACHE_TABLE, key),
            CoreStoreBucket::TlsCerts => read_bytes(&read_txn, TLS_CERTS_TABLE, key),
            CoreStoreBucket::HibernationState => {
                read_bytes(&read_txn, HIBERNATION_STATE_TABLE, key)
            }
            CoreStoreBucket::DataEvents => read_bytes(&read_txn, DATA_EVENTS_TABLE, key),
            CoreStoreBucket::VectorIndices => read_bytes(&read_txn, VECTOR_INDICES_TABLE, key),
            CoreStoreBucket::AuthzPurgeOutbox => {
                read_bytes(&read_txn, AUTHZ_PURGE_OUTBOX_TABLE, key)
            }
            CoreStoreBucket::DataMutationOutbox => {
                read_bytes(&read_txn, DATA_MUTATION_OUTBOX_TABLE, key)
            }
            CoreStoreBucket::MeteringOutbox => read_bytes(&read_txn, METERING_OUTBOX_TABLE, key),
            CoreStoreBucket::ConfigUpdateOutbox => {
                read_bytes(&read_txn, CONFIG_UPDATE_OUTBOX_TABLE, key)
            }
        }
    }

    pub(crate) fn put(&self, bucket: CoreStoreBucket, key: &str, value: &[u8]) -> Result<()> {
        let write_txn = self
            .db
            .begin_write()
            .context("failed to begin core store write transaction")?;
        {
            match bucket {
                CoreStoreBucket::CwasmCache => {
                    let mut table = write_txn
                        .open_table(CWASM_CACHE_TABLE)
                        .context("failed to open cwasm_cache table")?;
                    table
                        .insert(key, value)
                        .context("failed to insert cwasm cache entry")?;
                }
                CoreStoreBucket::TlsCerts => {
                    let mut table = write_txn
                        .open_table(TLS_CERTS_TABLE)
                        .context("failed to open tls_certs table")?;
                    table
                        .insert(key, value)
                        .context("failed to insert tls cert entry")?;
                }
                CoreStoreBucket::HibernationState => {
                    let mut table = write_txn
                        .open_table(HIBERNATION_STATE_TABLE)
                        .context("failed to open hibernation_state table")?;
                    table
                        .insert(key, value)
                        .context("failed to insert hibernation state entry")?;
                }
                CoreStoreBucket::DataEvents => {
                    let mut table = write_txn
                        .open_table(DATA_EVENTS_TABLE)
                        .context("failed to open data_events table")?;
                    table
                        .insert(key, value)
                        .context("failed to insert data events entry")?;
                }
                CoreStoreBucket::VectorIndices => {
                    let mut table = write_txn
                        .open_table(VECTOR_INDICES_TABLE)
                        .context("failed to open vector_indices table")?;
                    table
                        .insert(key, value)
                        .context("failed to insert vector index entry")?;
                }
                CoreStoreBucket::AuthzPurgeOutbox => {
                    let mut table = write_txn
                        .open_table(AUTHZ_PURGE_OUTBOX_TABLE)
                        .context("failed to open authz_purge_outbox table")?;
                    table
                        .insert(key, value)
                        .context("failed to insert authz purge outbox entry")?;
                }
                CoreStoreBucket::DataMutationOutbox => {
                    let mut table = write_txn
                        .open_table(DATA_MUTATION_OUTBOX_TABLE)
                        .context("failed to open data_mutation_outbox table")?;
                    table
                        .insert(key, value)
                        .context("failed to insert data mutation outbox entry")?;
                }
                CoreStoreBucket::MeteringOutbox => {
                    let mut table = write_txn
                        .open_table(METERING_OUTBOX_TABLE)
                        .context("failed to open metering_outbox table")?;
                    table
                        .insert(key, value)
                        .context("failed to insert metering outbox entry")?;
                }
                CoreStoreBucket::ConfigUpdateOutbox => {
                    let mut table = write_txn
                        .open_table(CONFIG_UPDATE_OUTBOX_TABLE)
                        .context("failed to open config_update_outbox table")?;
                    table
                        .insert(key, value)
                        .context("failed to insert config update outbox entry")?;
                }
            };
        }
        write_txn
            .commit()
            .context("failed to commit core store write transaction")
    }

    pub(crate) fn delete(&self, bucket: CoreStoreBucket, key: &str) -> Result<()> {
        let write_txn = self
            .db
            .begin_write()
            .context("failed to begin core store delete transaction")?;
        {
            match bucket {
                CoreStoreBucket::CwasmCache => {
                    write_txn
                        .open_table(CWASM_CACHE_TABLE)
                        .context("failed to open cwasm_cache table")?
                        .remove(key)
                        .context("failed to delete cwasm cache entry")?;
                }
                CoreStoreBucket::TlsCerts => {
                    write_txn
                        .open_table(TLS_CERTS_TABLE)
                        .context("failed to open tls_certs table")?
                        .remove(key)
                        .context("failed to delete tls cert entry")?;
                }
                CoreStoreBucket::HibernationState => {
                    write_txn
                        .open_table(HIBERNATION_STATE_TABLE)
                        .context("failed to open hibernation_state table")?
                        .remove(key)
                        .context("failed to delete hibernation state entry")?;
                }
                CoreStoreBucket::DataEvents => {
                    write_txn
                        .open_table(DATA_EVENTS_TABLE)
                        .context("failed to open data_events table")?
                        .remove(key)
                        .context("failed to delete data events entry")?;
                }
                CoreStoreBucket::VectorIndices => {
                    write_txn
                        .open_table(VECTOR_INDICES_TABLE)
                        .context("failed to open vector_indices table")?
                        .remove(key)
                        .context("failed to delete vector index entry")?;
                }
                CoreStoreBucket::AuthzPurgeOutbox => {
                    write_txn
                        .open_table(AUTHZ_PURGE_OUTBOX_TABLE)
                        .context("failed to open authz_purge_outbox table")?
                        .remove(key)
                        .context("failed to delete authz purge outbox entry")?;
                }
                CoreStoreBucket::DataMutationOutbox => {
                    write_txn
                        .open_table(DATA_MUTATION_OUTBOX_TABLE)
                        .context("failed to open data_mutation_outbox table")?
                        .remove(key)
                        .context("failed to delete data mutation outbox entry")?;
                }
                CoreStoreBucket::MeteringOutbox => {
                    write_txn
                        .open_table(METERING_OUTBOX_TABLE)
                        .context("failed to open metering_outbox table")?
                        .remove(key)
                        .context("failed to delete metering outbox entry")?;
                }
                CoreStoreBucket::ConfigUpdateOutbox => {
                    write_txn
                        .open_table(CONFIG_UPDATE_OUTBOX_TABLE)
                        .context("failed to open config_update_outbox table")?
                        .remove(key)
                        .context("failed to delete config update outbox entry")?;
                }
            }
        }
        write_txn
            .commit()
            .context("failed to commit core store delete transaction")
    }

    /// Append a new event to one of the outbox-style tables. The key is generated
    /// monotonically from the wall clock so a range scan returns events in
    /// approximate insertion order. Producers don't need to coordinate; the worst
    /// collision case is a single re-try on the same nanosecond, which we resolve
    /// with a numeric suffix.
    pub(crate) fn append_outbox(&self, bucket: CoreStoreBucket, payload: &[u8]) -> Result<String> {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mut suffix: u32 = 0;
        loop {
            // 22-digit decimal nanosecond + 4-digit suffix sorts as bytes the same
            // way it sorts as a number, so a range scan is in insertion order.
            let key = format!("{nanos:022}-{suffix:04}");
            if self.get(bucket, &key)?.is_none() {
                self.put(bucket, &key, payload)?;
                return Ok(key);
            }
            suffix = suffix
                .checked_add(1)
                .context("outbox key suffix overflow")?;
        }
    }

    /// Drain up to `limit` rows from one of the outbox-style tables in insertion
    /// order. Each yielded row is `(key, payload)`; the caller is expected to call
    /// `delete(bucket, key)` once it has durably handled the row, matching the
    /// at-least-once delivery semantics the proposals call out.
    #[allow(dead_code)]
    pub(crate) fn peek_outbox(
        &self,
        bucket: CoreStoreBucket,
        limit: usize,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        let table = match bucket {
            CoreStoreBucket::AuthzPurgeOutbox => AUTHZ_PURGE_OUTBOX_TABLE,
            CoreStoreBucket::DataMutationOutbox => DATA_MUTATION_OUTBOX_TABLE,
            CoreStoreBucket::MeteringOutbox => METERING_OUTBOX_TABLE,
            CoreStoreBucket::ConfigUpdateOutbox => CONFIG_UPDATE_OUTBOX_TABLE,
            other => {
                return Err(anyhow::anyhow!(
                    "peek_outbox is only valid for outbox-style buckets; got {other:?}"
                ));
            }
        };
        let read_txn = self
            .db
            .begin_read()
            .context("failed to begin outbox read transaction")?;
        let table = read_txn
            .open_table(table)
            .context("failed to open outbox table for peek")?;
        let mut out = Vec::with_capacity(limit);
        for entry in table.iter().context("failed to iterate outbox")? {
            let (key, value) = entry.context("failed to read outbox entry")?;
            out.push((key.value().to_owned(), value.value().to_vec()));
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    pub(crate) fn snapshot_directory(&self, key: &str, source: &Path) -> Result<()> {
        let snapshot = DirectorySnapshot::capture(source)?;
        let payload =
            serde_json::to_vec(&snapshot).context("failed to serialize hibernation snapshot")?;
        self.put(CoreStoreBucket::HibernationState, key, &payload)
    }

    pub(crate) fn restore_directory(&self, key: &str, destination: &Path) -> Result<bool> {
        let Some(payload) = self.get(CoreStoreBucket::HibernationState, key)? else {
            return Ok(false);
        };
        let snapshot: DirectorySnapshot = serde_json::from_slice(&payload)
            .context("failed to deserialize hibernation snapshot")?;
        snapshot.restore(destination)?;
        self.delete(CoreStoreBucket::HibernationState, key)?;
        Ok(true)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn create_vector_index(
        &self,
        tenant_id: &str,
        index_name: &str,
        dim: usize,
        m: u32,
        ef_construction: u32,
    ) -> Result<()> {
        let key = vector_index_key(tenant_id, index_name)?;
        if self.get(CoreStoreBucket::VectorIndices, &key)?.is_some() {
            return Ok(());
        }
        let index = VectorIndex {
            dim,
            m,
            ef_construction,
            documents: Vec::new(),
        };
        self.put(
            CoreStoreBucket::VectorIndices,
            &key,
            &serde_json::to_vec(&index).context("failed to serialize vector index")?,
        )
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn upsert_vectors(
        &self,
        tenant_id: &str,
        index_name: &str,
        docs: Vec<VectorDocument>,
    ) -> Result<()> {
        let key = vector_index_key(tenant_id, index_name)?;
        let mut index = self.load_vector_index(&key)?;
        for doc in docs {
            validate_vector_dim(index.dim, &doc.embedding)?;
            if let Some(existing) = index.documents.iter_mut().find(|entry| entry.id == doc.id) {
                *existing = doc;
            } else {
                index.documents.push(doc);
            }
        }
        self.put(
            CoreStoreBucket::VectorIndices,
            &key,
            &serde_json::to_vec(&index).context("failed to serialize vector index")?,
        )
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn search_vectors(
        &self,
        tenant_id: &str,
        index_name: &str,
        query: &[f32],
        k: usize,
    ) -> Result<Vec<VectorSearchMatch>> {
        let key = vector_index_key(tenant_id, index_name)?;
        let index = self.load_vector_index(&key)?;
        validate_vector_dim(index.dim, query)?;
        let mut matches = index
            .documents
            .iter()
            .map(|doc| VectorSearchMatch {
                id: doc.id.clone(),
                score: cosine_similarity(&doc.embedding, query),
                payload: doc.payload.clone(),
            })
            .collect::<Vec<_>>();
        matches.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
        });
        matches.truncate(k.min(100));
        Ok(matches)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn remove_vector(
        &self,
        tenant_id: &str,
        index_name: &str,
        id: &str,
    ) -> Result<bool> {
        let key = vector_index_key(tenant_id, index_name)?;
        let mut index = self.load_vector_index(&key)?;
        let before = index.documents.len();
        index.documents.retain(|doc| doc.id != id);
        let removed = index.documents.len() != before;
        if removed {
            self.put(
                CoreStoreBucket::VectorIndices,
                &key,
                &serde_json::to_vec(&index).context("failed to serialize vector index")?,
            )?;
        }
        Ok(removed)
    }

    // ── KV-cache (LLM inference token cache, isolated per model) ────────────

    pub(crate) fn kv_cache_get(
        &self,
        model_ref: &str,
        tenant: &str,
        cache_key: &str,
    ) -> Result<Option<Vec<u8>>> {
        let key = kv_cache_key(model_ref, tenant, cache_key)?;
        let read_txn = self
            .db
            .begin_read()
            .context("failed to begin kv_cache read transaction")?;
        let table = read_txn
            .open_table(KV_CACHE_TABLE)
            .context("failed to open kv_cache table")?;
        let Some(raw) = table.get(key.as_str())? else {
            return Ok(None);
        };
        let entry: KvCacheEntry =
            serde_json::from_slice(raw.value()).context("failed to deserialize kv_cache entry")?;
        // Evict expired entries on read.
        if entry.is_expired() {
            drop(raw);
            drop(table);
            drop(read_txn);
            let _ = self.kv_cache_delete(model_ref, tenant, cache_key);
            return Ok(None);
        }
        Ok(Some(entry.value))
    }

    pub(crate) fn kv_cache_put(
        &self,
        model_ref: &str,
        tenant: &str,
        cache_key: &str,
        value: &[u8],
        ttl_seconds: Option<u64>,
    ) -> Result<()> {
        let key = kv_cache_key(model_ref, tenant, cache_key)?;
        let entry = KvCacheEntry::new(value.to_vec(), ttl_seconds);
        let payload = serde_json::to_vec(&entry).context("failed to serialize kv_cache entry")?;
        let write_txn = self
            .db
            .begin_write()
            .context("failed to begin kv_cache write transaction")?;
        {
            write_txn
                .open_table(KV_CACHE_TABLE)
                .context("failed to open kv_cache table")?
                .insert(key.as_str(), payload.as_slice())
                .context("failed to insert kv_cache entry")?;
        }
        write_txn
            .commit()
            .context("failed to commit kv_cache write transaction")
    }

    pub(crate) fn kv_cache_delete(
        &self,
        model_ref: &str,
        tenant: &str,
        cache_key: &str,
    ) -> Result<()> {
        let key = kv_cache_key(model_ref, tenant, cache_key)?;
        let write_txn = self
            .db
            .begin_write()
            .context("failed to begin kv_cache delete transaction")?;
        {
            write_txn
                .open_table(KV_CACHE_TABLE)
                .context("failed to open kv_cache table")?
                .remove(key.as_str())
                .context("failed to delete kv_cache entry")?;
        }
        write_txn
            .commit()
            .context("failed to commit kv_cache delete transaction")
    }

    /// Delete all cache entries whose key begins with `{model_ref}/`.
    /// Returns the number of evicted entries.
    pub(crate) fn kv_cache_evict_model(&self, model_ref: &str) -> Result<usize> {
        let prefix = format!("{}/", sanitize_kv_key_part(model_ref, "model_ref")?);
        // Upper bound: replace the trailing '/' (0x2F) with '0' (0x30).
        let upper = format!("{}0", sanitize_kv_key_part(model_ref, "model_ref")?);

        let write_txn = self
            .db
            .begin_write()
            .context("failed to begin kv_cache evict transaction")?;
        let evicted;
        {
            let mut table = write_txn
                .open_table(KV_CACHE_TABLE)
                .context("failed to open kv_cache table")?;
            let keys_to_delete: Vec<String> = table
                .range(prefix.as_str()..upper.as_str())?
                .filter_map(|entry| entry.ok())
                .filter(|(k, _)| k.value().starts_with(prefix.as_str()))
                .map(|(k, _)| k.value().to_owned())
                .collect();
            evicted = keys_to_delete.len();
            for key in &keys_to_delete {
                table
                    .remove(key.as_str())
                    .context("failed to remove kv_cache entry during eviction")?;
            }
        }
        write_txn
            .commit()
            .context("failed to commit kv_cache eviction transaction")?;
        Ok(evicted)
    }

    /// Count of live (non-expired) entries for `model_ref` and total bytes stored.
    pub(crate) fn kv_cache_stats(&self, model_ref: &str) -> Result<KvCacheStats> {
        let prefix = format!("{}/", sanitize_kv_key_part(model_ref, "model_ref")?);
        let upper = format!("{}0", sanitize_kv_key_part(model_ref, "model_ref")?);
        let read_txn = self
            .db
            .begin_read()
            .context("failed to begin kv_cache stats transaction")?;
        let table = read_txn
            .open_table(KV_CACHE_TABLE)
            .context("failed to open kv_cache table")?;
        let mut entry_count: usize = 0;
        let mut total_bytes: usize = 0;
        let mut expired_count: usize = 0;
        for result in table.range(prefix.as_str()..upper.as_str())? {
            let (k, v) = result?;
            if !k.value().starts_with(prefix.as_str()) {
                break;
            }
            if let Ok(entry) = serde_json::from_slice::<KvCacheEntry>(v.value()) {
                if entry.is_expired() {
                    expired_count += 1;
                } else {
                    entry_count += 1;
                    total_bytes += entry.value.len();
                }
            }
        }
        Ok(KvCacheStats {
            entry_count,
            total_bytes,
            expired_count,
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    fn load_vector_index(&self, key: &str) -> Result<VectorIndex> {
        let Some(payload) = self.get(CoreStoreBucket::VectorIndices, key)? else {
            anyhow::bail!("vector index `{key}` does not exist");
        };
        serde_json::from_slice(&payload).context("failed to deserialize vector index")
    }

    // ── Guest KV-partition operations ────────────────────────────────────────

    pub(crate) fn kv_partition_get(&self, table_name: &str, key: &str) -> Result<Option<Vec<u8>>> {
        let table_key = format!("kv_partition::{table_name}");
        let table_def: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new(&table_key);
        let read_txn = self
            .db
            .begin_read()
            .context("kv_partition_get: failed to begin read transaction")?;
        let table = match read_txn.open_table(table_def) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(None),
            Err(e) => return Err(e).context("kv_partition_get: failed to open table"),
        };
        Ok(table
            .get(key)
            .context("kv_partition_get: read failed")?
            .map(|v| v.value().to_owned()))
    }

    pub(crate) fn kv_partition_set(&self, table_name: &str, key: &str, value: &[u8]) -> Result<()> {
        let table_key = format!("kv_partition::{table_name}");
        let table_def: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new(&table_key);
        let write_txn = self
            .db
            .begin_write()
            .context("kv_partition_set: failed to begin write transaction")?;
        {
            let mut table = write_txn
                .open_table(table_def)
                .context("kv_partition_set: failed to open table")?;
            table
                .insert(key, value)
                .context("kv_partition_set: insert failed")?;
        }
        write_txn
            .commit()
            .context("kv_partition_set: failed to commit")
    }

    pub(crate) fn kv_partition_delete(&self, table_name: &str, key: &str) -> Result<()> {
        let table_key = format!("kv_partition::{table_name}");
        let table_def: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new(&table_key);
        let write_txn = self
            .db
            .begin_write()
            .context("kv_partition_delete: failed to begin write transaction")?;
        {
            let mut table = write_txn
                .open_table(table_def)
                .context("kv_partition_delete: failed to open table")?;
            table
                .remove(key)
                .context("kv_partition_delete: remove failed")?;
        }
        write_txn
            .commit()
            .context("kv_partition_delete: failed to commit")
    }

    #[allow(dead_code)]
    pub(crate) fn kv_partition_batch_set(
        &self,
        table_name: &str,
        entries: &[(String, Vec<u8>)],
    ) -> Result<()> {
        let table_key = format!("kv_partition::{table_name}");
        let table_def: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new(&table_key);
        let write_txn = self
            .db
            .begin_write()
            .context("kv_partition_batch_set: failed to begin write transaction")?;
        {
            let mut table = write_txn
                .open_table(table_def)
                .context("kv_partition_batch_set: failed to open table")?;
            for (key, value) in entries {
                table
                    .insert(key.as_str(), value.as_slice())
                    .context("kv_partition_batch_set: insert failed")?;
            }
        }
        write_txn
            .commit()
            .context("kv_partition_batch_set: failed to commit")
    }

    #[allow(dead_code)]
    pub(crate) fn kv_partition_get_range(
        &self,
        table_name: &str,
        start_key: &str,
        end_key: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<(String, Vec<u8>)>> {
        let table_key = format!("kv_partition::{table_name}");
        let table_def: redb::TableDefinition<&str, &[u8]> = redb::TableDefinition::new(&table_key);
        let read_txn = self
            .db
            .begin_read()
            .context("kv_partition_get_range: failed to begin read transaction")?;
        let table = match read_txn.open_table(table_def) {
            Ok(t) => t,
            Err(redb::TableError::TableDoesNotExist(_)) => return Ok(Vec::new()),
            Err(e) => return Err(e).context("kv_partition_get_range: failed to open table"),
        };
        let results: std::result::Result<Vec<_>, _> = table
            .range(start_key..end_key)
            .context("kv_partition_get_range: range scan failed")?
            .skip(offset as usize)
            .take(limit as usize)
            .map(|result| result.map(|(k, v)| (k.value().to_owned(), v.value().to_owned())))
            .collect();
        results.context("kv_partition_get_range: failed to collect range results")
    }
}

// ── Semantic graph store (hexastore) ─────────────────────────────────────────

#[allow(dead_code)]
const GRAPH_SEP: u8 = b'\0';

#[allow(dead_code)]
fn graph_spo_key(subject: &str, predicate: &str, object: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(subject.len() + predicate.len() + object.len() + 2);
    k.extend_from_slice(subject.as_bytes());
    k.push(GRAPH_SEP);
    k.extend_from_slice(predicate.as_bytes());
    k.push(GRAPH_SEP);
    k.extend_from_slice(object.as_bytes());
    k
}

#[allow(dead_code)]
fn graph_osp_key(object: &str, subject: &str, predicate: &str) -> Vec<u8> {
    let mut k = Vec::with_capacity(object.len() + subject.len() + predicate.len() + 2);
    k.extend_from_slice(object.as_bytes());
    k.push(GRAPH_SEP);
    k.extend_from_slice(subject.as_bytes());
    k.push(GRAPH_SEP);
    k.extend_from_slice(predicate.as_bytes());
    k
}

#[allow(dead_code)]
fn graph_spo_prefix_range(subject: &str, predicate: &str) -> (Vec<u8>, Vec<u8>) {
    let mut start = Vec::with_capacity(subject.len() + predicate.len() + 2);
    start.extend_from_slice(subject.as_bytes());
    start.push(GRAPH_SEP);
    start.extend_from_slice(predicate.as_bytes());
    start.push(GRAPH_SEP);
    let mut end = start.clone();
    if let Some(last) = end.last_mut() {
        *last = last.saturating_add(1);
    } else {
        end.push(1);
    }
    (start, end)
}

/// An edge in the semantic graph.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct GraphEdge {
    pub(crate) subject: String,
    pub(crate) predicate: String,
    pub(crate) object: String,
    pub(crate) properties: String,
}

#[allow(dead_code)]
const GRAPH_TRAVERSE_LIMIT: usize = 10_000;

#[allow(dead_code)]
impl CoreStore {
    fn graph_spo_table(name: &str) -> String {
        format!("graph_{name}_spo")
    }
    fn graph_osp_table(name: &str) -> String {
        format!("graph_{name}_osp")
    }

    pub(crate) fn graph_add_edges(&self, graph_name: &str, edges: &[GraphEdge]) -> Result<()> {
        let spo_name = Self::graph_spo_table(graph_name);
        let osp_name = Self::graph_osp_table(graph_name);
        let spo_def: redb::TableDefinition<&[u8], &str> = redb::TableDefinition::new(&spo_name);
        let osp_def: redb::TableDefinition<&[u8], &str> = redb::TableDefinition::new(&osp_name);
        let wtxn = self
            .db
            .begin_write()
            .context("graph_add_edges: begin write")?;
        {
            let mut spo = wtxn
                .open_table(spo_def)
                .context("graph_add_edges: open SPO")?;
            let mut osp = wtxn
                .open_table(osp_def)
                .context("graph_add_edges: open OSP")?;
            for e in edges {
                spo.insert(
                    graph_spo_key(&e.subject, &e.predicate, &e.object).as_slice(),
                    e.properties.as_str(),
                )
                .context("graph_add_edges: insert SPO")?;
                osp.insert(
                    graph_osp_key(&e.object, &e.subject, &e.predicate).as_slice(),
                    e.properties.as_str(),
                )
                .context("graph_add_edges: insert OSP")?;
            }
        }
        wtxn.commit().context("graph_add_edges: commit")
    }

    pub(crate) fn graph_delete_edges(&self, graph_name: &str, edges: &[GraphEdge]) -> Result<()> {
        let spo_name = Self::graph_spo_table(graph_name);
        let osp_name = Self::graph_osp_table(graph_name);
        let spo_def: redb::TableDefinition<&[u8], &str> = redb::TableDefinition::new(&spo_name);
        let osp_def: redb::TableDefinition<&[u8], &str> = redb::TableDefinition::new(&osp_name);
        let wtxn = self
            .db
            .begin_write()
            .context("graph_delete_edges: begin write")?;
        {
            let mut spo = wtxn
                .open_table(spo_def)
                .context("graph_delete_edges: open SPO")?;
            let mut osp = wtxn
                .open_table(osp_def)
                .context("graph_delete_edges: open OSP")?;
            for e in edges {
                spo.remove(graph_spo_key(&e.subject, &e.predicate, &e.object).as_slice())
                    .context("graph_delete_edges: remove SPO")?;
                osp.remove(graph_osp_key(&e.object, &e.subject, &e.predicate).as_slice())
                    .context("graph_delete_edges: remove OSP")?;
            }
        }
        wtxn.commit().context("graph_delete_edges: commit")
    }

    pub(crate) fn graph_traverse(
        &self,
        graph_name: &str,
        subject: &str,
        predicate: &str,
        depth: u32,
    ) -> Result<Vec<String>> {
        if depth == 0 {
            return Ok(Vec::new());
        }
        let spo_name = Self::graph_spo_table(graph_name);
        let spo_def: redb::TableDefinition<&[u8], &str> = redb::TableDefinition::new(&spo_name);
        let rtxn = self.db.begin_read().context("graph_traverse: begin read")?;
        let spo = rtxn
            .open_table(spo_def)
            .context("graph_traverse: open SPO")?;

        let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();
        visited.insert(subject.to_owned());
        let mut queue: std::collections::VecDeque<(String, u32)> =
            std::collections::VecDeque::new();
        queue.push_back((subject.to_owned(), 0));
        let mut results: Vec<String> = Vec::new();

        while let Some((node, cur_depth)) = queue.pop_front() {
            if cur_depth >= depth {
                continue;
            }
            let (start, end) = graph_spo_prefix_range(&node, predicate);
            let range = spo
                .range::<&[u8]>(start.as_slice()..end.as_slice())
                .context("graph_traverse: range scan")?;
            for item in range {
                let (key_guard, _) = item.context("graph_traverse: read row")?;
                let key = key_guard.value();
                let prefix_len = node.len() + 1 + predicate.len() + 1;
                if key.len() <= prefix_len {
                    continue;
                }
                let object = match std::str::from_utf8(&key[prefix_len..]) {
                    Ok(s) => s.to_owned(),
                    Err(_) => continue,
                };
                if results.len() >= GRAPH_TRAVERSE_LIMIT {
                    return Ok(results);
                }
                if visited.insert(object.clone()) {
                    results.push(object.clone());
                    queue.push_back((object, cur_depth + 1));
                }
            }
        }
        Ok(results)
    }
}

// ── KV-cache support types and helpers ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct KvCacheEntry {
    pub(crate) value: Vec<u8>,
    /// Unix timestamp (seconds) after which this entry is considered stale.
    /// `None` means the entry never expires.
    pub(crate) expires_at: Option<u64>,
}

impl KvCacheEntry {
    fn new(value: Vec<u8>, ttl_seconds: Option<u64>) -> Self {
        let expires_at = ttl_seconds.map(|ttl| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
                .saturating_add(ttl)
        });
        Self { value, expires_at }
    }

    fn is_expired(&self) -> bool {
        let Some(expires_at) = self.expires_at else {
            return false;
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        now >= expires_at
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct KvCacheStats {
    pub(crate) entry_count: usize,
    pub(crate) total_bytes: usize,
    pub(crate) expired_count: usize,
}

/// Compose a storage key: `{model_ref}/{tenant}/{cache_key}`.
/// Model_ref and tenant may not contain `/` to keep prefix scans unambiguous.
fn kv_cache_key(model_ref: &str, tenant: &str, cache_key: &str) -> Result<String> {
    let model = sanitize_kv_key_part(model_ref, "model_ref")?;
    let t = sanitize_kv_key_part(tenant, "tenant")?;
    if cache_key.is_empty() {
        anyhow::bail!("kv cache_key must not be empty");
    }
    Ok(format!("{model}/{t}/{cache_key}"))
}

fn sanitize_kv_key_part(value: &str, label: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains('\\') {
        anyhow::bail!(
            "invalid kv-cache {label} `{value}`: must be non-empty and contain no slashes"
        );
    }
    Ok(trimmed.to_owned())
}

#[cfg_attr(not(test), allow(dead_code))]
fn vector_index_key(tenant_id: &str, index_name: &str) -> Result<String> {
    let tenant = sanitize_vector_key_part(tenant_id, "tenant id")?;
    let index = sanitize_vector_key_part(index_name, "index name")?;
    Ok(format!("{tenant}/{index}"))
}

#[cfg_attr(not(test), allow(dead_code))]
fn sanitize_vector_key_part(value: &str, label: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
    {
        anyhow::bail!("invalid vector {label} `{value}`");
    }
    Ok(trimmed.to_owned())
}

#[cfg_attr(not(test), allow(dead_code))]
fn validate_vector_dim(expected: usize, embedding: &[f32]) -> Result<()> {
    if expected == 0 || embedding.len() != expected {
        anyhow::bail!(
            "vector dimension mismatch: expected {expected}, got {}",
            embedding.len()
        );
    }
    Ok(())
}

#[cfg_attr(not(test), allow(dead_code))]
fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    let mut dot = 0.0_f32;
    let mut left_norm = 0.0_f32;
    let mut right_norm = 0.0_f32;
    for (left, right) in left.iter().zip(right.iter()) {
        dot += left * right;
        left_norm += left * left;
        right_norm += right * right;
    }
    if left_norm == 0.0 || right_norm == 0.0 {
        return 0.0;
    }
    dot / (left_norm.sqrt() * right_norm.sqrt())
}

fn read_bytes(
    txn: &redb::ReadTransaction,
    table: TableDefinition<&str, &[u8]>,
    key: &str,
) -> Result<Option<Vec<u8>>> {
    let table = txn
        .open_table(table)
        .context("failed to open core store table")?;
    Ok(table.get(key)?.map(|value| value.value().to_vec()))
}

impl DirectorySnapshot {
    fn capture(source: &Path) -> Result<Self> {
        let mut entries = Vec::new();
        capture_directory_entries(source, source, &mut entries)?;
        Ok(Self { entries })
    }

    fn restore(&self, destination: &Path) -> Result<()> {
        remove_path_if_exists(destination)?;
        fs::create_dir_all(destination).with_context(|| {
            format!(
                "failed to create hibernation restore directory `{}`",
                destination.display()
            )
        })?;

        for entry in &self.entries {
            let target_path = destination.join(sanitize_relative_path(&entry.relative_path)?);
            match entry.kind {
                DirectoryEntryKind::Directory => {
                    fs::create_dir_all(&target_path).with_context(|| {
                        format!(
                            "failed to recreate hibernated directory `{}`",
                            target_path.display()
                        )
                    })?;
                }
                DirectoryEntryKind::File => {
                    if let Some(parent) = target_path.parent() {
                        fs::create_dir_all(parent).with_context(|| {
                            format!(
                                "failed to create hibernation restore parent `{}`",
                                parent.display()
                            )
                        })?;
                    }
                    fs::write(&target_path, &entry.body).with_context(|| {
                        format!(
                            "failed to restore hibernated file `{}`",
                            target_path.display()
                        )
                    })?;
                }
            }
        }

        Ok(())
    }
}

fn capture_directory_entries(
    base: &Path,
    current: &Path,
    entries: &mut Vec<DirectoryEntry>,
) -> Result<()> {
    if !current.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(current)
        .with_context(|| format!("failed to read hibernation source `{}`", current.display()))?
    {
        let entry = entry.with_context(|| {
            format!(
                "failed to read hibernation entry inside `{}`",
                current.display()
            )
        })?;
        let path = entry.path();
        let relative_path = path
            .strip_prefix(base)
            .expect("captured path should stay within snapshot root")
            .to_string_lossy()
            .replace('\\', "/");
        let metadata = entry.metadata().with_context(|| {
            format!(
                "failed to read metadata for hibernation path `{}`",
                path.display()
            )
        })?;

        if metadata.is_dir() {
            entries.push(DirectoryEntry {
                relative_path,
                kind: DirectoryEntryKind::Directory,
                body: Vec::new(),
            });
            capture_directory_entries(base, &path, entries)?;
        } else {
            entries.push(DirectoryEntry {
                relative_path,
                kind: DirectoryEntryKind::File,
                body: fs::read(&path).with_context(|| {
                    format!("failed to read hibernation file `{}`", path.display())
                })?,
            });
        }
    }

    Ok(())
}

fn sanitize_relative_path(value: &str) -> Result<PathBuf> {
    let path = Path::new(value);
    let mut sanitized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => sanitized.push(part),
            Component::CurDir => {}
            _ => anyhow::bail!("hibernation snapshot contains an invalid relative path"),
        }
    }
    Ok(sanitized)
}

fn remove_path_if_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to read metadata for `{}`", path.display()))?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove directory `{}`", path.display()))?;
    } else {
        fs::remove_file(path)
            .with_context(|| format!("failed to remove file `{}`", path.display()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn open_test_store() -> CoreStore {
        let path = std::env::temp_dir().join(format!("tachyon-store-{}.db", Uuid::new_v4()));
        CoreStore::open(&path).expect("test store should open")
    }

    #[test]
    fn directory_snapshot_round_trips_nested_files() {
        let store = open_test_store();
        let source = std::env::temp_dir().join(format!("tachyon-src-{}", Uuid::new_v4()));
        let destination = std::env::temp_dir().join(format!("tachyon-dst-{}", Uuid::new_v4()));
        fs::create_dir_all(source.join("nested")).expect("source directory should exist");
        fs::write(source.join("nested").join("state.txt"), "hibernated")
            .expect("source file should be written");

        store
            .snapshot_directory("route:volume", &source)
            .expect("snapshot should be stored");
        remove_path_if_exists(&source).expect("source should be removed for restore test");

        let restored = store
            .restore_directory("route:volume", &destination)
            .expect("snapshot should restore");

        assert!(restored, "snapshot should exist");
        assert_eq!(
            fs::read_to_string(destination.join("nested").join("state.txt"))
                .expect("restored file should be readable"),
            "hibernated"
        );
        assert!(
            store
                .get(CoreStoreBucket::HibernationState, "route:volume")
                .expect("hibernation entry lookup should succeed")
                .is_none(),
            "hibernation entry should be deleted after restore"
        );
    }

    #[test]
    fn vector_index_upserts_searches_and_removes_documents() {
        let store = open_test_store();
        store
            .create_vector_index("tenant-a", "kb", 3, 16, 200)
            .expect("vector index should be created");
        store
            .upsert_vectors(
                "tenant-a",
                "kb",
                vec![
                    VectorDocument {
                        id: "doc-a".to_owned(),
                        embedding: vec![1.0, 0.0, 0.0],
                        payload: Some(b"alpha".to_vec()),
                    },
                    VectorDocument {
                        id: "doc-b".to_owned(),
                        embedding: vec![0.0, 1.0, 0.0],
                        payload: Some(b"beta".to_vec()),
                    },
                ],
            )
            .expect("vectors should upsert");

        let matches = store
            .search_vectors("tenant-a", "kb", &[0.9, 0.1, 0.0], 1)
            .expect("vector search should succeed");
        assert_eq!(matches[0].id, "doc-a");
        assert_eq!(matches[0].payload.as_deref(), Some(&b"alpha"[..]));

        assert!(store
            .remove_vector("tenant-a", "kb", "doc-a")
            .expect("vector remove should succeed"));
        let matches = store
            .search_vectors("tenant-a", "kb", &[0.9, 0.1, 0.0], 5)
            .expect("vector search should succeed after delete");
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].id, "doc-b");
    }

    #[test]
    fn smart_compression_skips_precompressed_media() {
        assert!(!should_compress_blob(b"\x89PNG\r\n"));
        assert!(!should_compress_blob(b"\xFF\xD8\xFF\xE0"));
        assert!(should_compress_blob(b"{\"json\":true}"));
    }

    #[test]
    fn query_cache_reuses_smolvm_mutation_result() {
        let cache = BaasQueryCache::default();
        let first = cache
            .get_or_insert_with("select * from docs", "allow if true", || {
                Ok("sanitized".into())
            })
            .expect("cache should build");
        let second = cache
            .get_or_insert_with("select * from docs", "allow if true", || {
                Ok("different".into())
            })
            .expect("cache should hit");

        assert_eq!(first, "sanitized");
        assert_eq!(second, "sanitized");
    }

    #[test]
    fn stale_record_invokes_schema_shim() {
        let transformed = transform_read_if_stale(
            2,
            VersionedRecord {
                schema_version: 1,
                payload: b"old".to_vec(),
            },
            |version, payload| {
                assert_eq!(version, 1);
                Ok([payload, b"-new"].concat())
            },
        )
        .expect("shim should transform");

        assert_eq!(transformed, b"old-new");
    }

    #[test]
    fn split_brain_conflict_uses_resolver() {
        let merged = resolve_conflict_with(
            ConflictState {
                key: "k".to_owned(),
                local_value: b"left".to_vec(),
                remote_value: b"right".to_vec(),
                local_clock: 7,
                remote_clock: 3,
            },
            |conflict| Ok([&conflict.local_value[..], b"+", &conflict.remote_value[..]].concat()),
        )
        .expect("conflict should resolve");

        assert_eq!(merged, b"left+right");
    }

    #[test]
    fn pushdown_filtered_scan_prunes_rows_before_limit() {
        let rows = vec![
            (b"a1".to_vec(), b"drop".to_vec()),
            (b"a2".to_vec(), b"keep".to_vec()),
            (b"a3".to_vec(), b"keep".to_vec()),
        ];
        let options = ScanOptions {
            start_key: b"a".to_vec(),
            end_key: b"b".to_vec(),
            limit: 1,
            pushdown_filter_wasm: Some(vec![b'p']),
        };

        let result = execute_filtered_scan(rows, &options, |_, value, filter| {
            Ok(filter.is_some() && value.ends_with(b"p"))
        })
        .expect("pushdown scan should succeed");

        assert_eq!(result, vec![(b"a1".to_vec(), b"drop".to_vec())]);
        let config = pushdown_wasmtime_config();
        assert!(format!("{config:?}").contains("Config"));
    }
}
