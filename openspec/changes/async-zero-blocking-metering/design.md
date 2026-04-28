# Design: Fuel and Async Event Flow

## 1. Wasmtime Fuel Configuration (`core-host/src/store/mod.rs`)
Enable fuel consumption in the Wasmtime engine configuration.

```rust
let mut config = wasmtime::Config::new();
// Enable fuel consumption tracking
config.consume_fuel(true);
```

When creating a `Store` for an invocation, grant it a large amount of fuel (or infinite if just tracking).
```rust
let mut store = wasmtime::Store::new(&engine, state);
// Add fuel so the module can run
store.add_fuel(u64::MAX).unwrap();
```

## 2. Extraction and Emission (`core-host/src/server_h3.rs` or dispatcher)
After the `instance.get_typed_func(...).call_async()` finishes:

```rust
// Calculate fuel consumed
let fuel_consumed = store.fuel_consumed().unwrap_or(0);

// Fire and forget: send to the internal event router
let event = UsageEvent {
    tenant_id: request.tenant_id,
    module: module_id,
    fuel: fuel_consumed,
};
event_bus.send_async("tachyon.telemetry.usage", event);
```

## 3. Background Metering (`systems/system-faas-metering`)
This module no longer exposes a synchronous API to the host. Instead, it subscribes to the event bus.
- It uses a `HashMap<TenantId, u64>` to aggregate fuel locally.
- Every `flush_interval` (e.g., 60s), it writes the sums to the database and clears the local map.