use anyhow::{Context, Result};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Ordering,
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

const CWASM_CACHE_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("cwasm_cache");
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

#[derive(Clone)]
pub(crate) struct CoreStore {
    db: Arc<Database>,
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
        }
        write_txn
            .commit()
            .context("failed to commit core store initialization transaction")
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

    #[cfg_attr(not(test), allow(dead_code))]
    fn load_vector_index(&self, key: &str) -> Result<VectorIndex> {
        let Some(payload) = self.get(CoreStoreBucket::VectorIndices, key)? else {
            anyhow::bail!("vector index `{key}` does not exist");
        };
        serde_json::from_slice(&payload).context("failed to deserialize vector index")
    }
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
}
