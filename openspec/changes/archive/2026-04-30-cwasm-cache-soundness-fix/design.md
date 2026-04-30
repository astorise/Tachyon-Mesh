# Design: Engine Compatibility Binding

## 1. Key Generation Update (`core-host/src/main.rs`)
Locate the Cwasm caching logic (around line 9108 as identified in the audit). Update the key generation to include the compatibility hash.

```rust
// Old vulnerable logic
// let cache_key = format!("{}:{}:{}:{}", kind, scope, path, wasm_sha256);

// New secure logic
let engine_hash = engine.precompile_compatibility_hash();
let cache_key = format!("{}:{}:{}:{}:{}", kind, scope, path, wasm_sha256, engine_hash);
```

## 2. Boot-Time Cache Purge (`core-host/src/main.rs`)
During the host bootstrap phase, verify the engine hash against the database metadata.

```rust
const METADATA_TABLE: TableDefinition<&str, &str> = TableDefinition::new("metadata");
const CWASM_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("cwasm_cache");

pub fn secure_cache_bootstrap(db: &Database, engine: &wasmtime::Engine) -> Result<()> {
    let current_hash = engine.precompile_compatibility_hash();
    
    let write_txn = db.begin_write()?;
    {
        let mut metadata_table = write_txn.open_table(METADATA_TABLE)?;
        let stored_hash = metadata_table.get("engine_hash")?;
        
        if let Some(stored) = stored_hash {
            if stored.value() != current_hash {
                tracing::warn!("Wasmtime engine updated. Purging stale Cwasm cache to prevent UB.");
                // Drop and recreate the Cwasm table to reclaim space
                write_txn.delete_table(CWASM_TABLE)?;
                write_txn.open_table(CWASM_TABLE)?;
            }
        }
        
        // Save the current hash for future boots
        metadata_table.insert("engine_hash", current_hash.as_str())?;
    }
    write_txn.commit()?;
    Ok(())
}
```