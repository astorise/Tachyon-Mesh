# Design: Invalidation Mechanics

## 1. Event Emission (`systems/system-faas-authz/src/lib.rs`)
The Wasm module needs to emit a standardized event payload whenever a mutating function is successfully called.

**Event Payload Schema (JSON):**
```json
{
  "action": "revoke_token",
  "target_type": "token_hash",
  "target_id": "a1b2c3d4..."
}
```
*Note: Depending on the implemented IPC event system (e.g., `tachyon.events.emit`), the authz module will broadcast this to a reserved system topic.*

## 2. Core Host Subscriber (`core-host/src/auth.rs` & `data_events.rs`)
The `core-host` currently holds a cache, likely a `moka::future::Cache` or a `DashMap` wrapped in an `Arc<RwLock<...>>`.

**Workflow:**
- During `core-host` boot, spawn a Tokio task that subscribes to the `system:authz:events` internal channel.
- When an event is received, parse the `target_type` and `target_id`.
- If `target_type == "token_hash"`, call `auth_cache.remove(&target_id)`.
- If `target_type == "user_id"`, iterate/query the cache to remove all cached decisions associated with that user.

## 3. Fallback TTL
As a defense-in-depth measure, the absolute TTL on the cache should remain (e.g., 5 minutes). If the event bus drops a message (which shouldn't happen via IPC, but architecture must account for it), the access is eventually revoked natively.