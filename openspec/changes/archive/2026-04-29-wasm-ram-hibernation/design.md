# Design: Hibernation Lifecycle

## 1. Instance State Tracking (`core-host/src/store/pool.rs`)
Modify the pool data structure to track activity and state.

```rust
pub enum InstanceState {
    Active { 
        instance: wasmtime::InstancePre<StoreState>, 
        last_accessed: std::time::Instant 
    },
    Hibernated { 
        snapshot_path: std::path::PathBuf 
    },
}
```

## 2. Background Hibernation Task (`core-host/src/hibernation.rs`)
Create a Tokio background loop that runs every 60 seconds:
- Iterate over the FaaS pool.
- If `state == Active` and `now.duration_since(last_accessed) > 5 minutes`:
    - Extract the Wasm memory (using `wasmtime::Memory::data(&store)`).
    - Write the byte array to `/var/lib/tachyon/snapshots/<module_id>.snap`.
    - Change state to `Hibernated`.
    - Drop the `InstancePre` to free the RAM.

## 3. The "Thaw" Process
When a request is dispatched to the FaaS router:
- Check the `InstanceState`.
- If `Hibernated`:
    - Read the `.snap` file into a `Vec<u8>`.
    - Re-instantiate the module from the `system-faas-registry` cache.
    - Write the `Vec<u8>` back into the new instance's linear memory (`Memory::write(&mut store, 0, &snapshot_data)`).
    - Update the state back to `Active` and update `last_accessed`.
- Execute the request.