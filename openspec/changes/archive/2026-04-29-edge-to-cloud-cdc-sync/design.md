# Design: Asynchronous CDC Architecture

## 1. Schema Update (`core-host/src/main.rs`)
Update the persistence resource definition to make CDC opt-in.

```rust
pub struct KeyValueStoreConfig {
    pub name: String,
    // ...
    #[serde(default)]
    pub sync_to_cloud: bool, // Defaults to false
}
```

## 2. Event Emission (`core-host/src/system_storage.rs`)
In the local KV or VFS write logic:

```rust
// 1. Perform the fast local write
local_db.insert(&key, &value)?;

// 2. If CDC is enabled, emit a fire-and-forget event
if config.sync_to_cloud {
    let payload = CdcEvent {
        resource: config.name.clone(),
        operation: "INSERT".to_string(),
        key,
        value_hash: compute_hash(&value), // Or send the full value based on policy
        timestamp: std::time::SystemTime::now(),
    };
    event_bus.send_async("tachyon.data.mutation", payload);
}
```

## 3. The CDC System FaaS (`systems/system-faas-cdc`)
This module handles the complex networking logic:
- **Ingestion:** Listens to `tachyon.data.mutation`.
- **Local Spooling:** Writes the events to a lightweight local queue (e.g., an append-only file or a dedicated SQLite table) to survive host reboots while offline.
- **Draining Task:** A Tokio interval task reads the spool. It attempts an HTTP/3 or gRPC call to the Cloud.
    - If successful: Mark events as acknowledged and delete from local spool.
    - If failed (Timeout/No Route): Back off and try again later.