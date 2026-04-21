use anyhow::{Context, Result};
use redb::{Database, TableDefinition};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

const CWASM_CACHE_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("cwasm_cache");
const TLS_CERTS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("tls_certs");
const HIBERNATION_STATE_TABLE: TableDefinition<&str, &[u8]> =
    TableDefinition::new("hibernation_state");
const DATA_EVENTS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("data_events");
const ASSET_REGISTRY_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("asset_registry");

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
    AssetRegistry,
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
                .open_table(ASSET_REGISTRY_TABLE)
                .context("failed to open asset_registry table")?;
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
            CoreStoreBucket::AssetRegistry => read_bytes(&read_txn, ASSET_REGISTRY_TABLE, key),
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
                CoreStoreBucket::AssetRegistry => {
                    let mut table = write_txn
                        .open_table(ASSET_REGISTRY_TABLE)
                        .context("failed to open asset_registry table")?;
                    table
                        .insert(key, value)
                        .context("failed to insert asset registry entry")?;
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
                CoreStoreBucket::AssetRegistry => {
                    write_txn
                        .open_table(ASSET_REGISTRY_TABLE)
                        .context("failed to open asset_registry table")?
                        .remove(key)
                        .context("failed to delete asset registry entry")?;
                }
            }
        }
        write_txn
            .commit()
            .context("failed to commit core store delete transaction")
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
}
