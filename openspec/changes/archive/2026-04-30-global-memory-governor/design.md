# Design: RSS Polling and Event Bus



## 1. The Governor Task (`core-host/src/runtime/governor.rs`)
A lightweight Tokio background task that wakes up every 500ms.
```rust
pub async fn memory_governor_loop(event_bus: Arc<EventBus>, limit_mb: u64) {
    let mut interval = tokio::time::interval(Duration::from_millis(500));
    loop {
        interval.tick().await;
        let current_rss_mb = fetch_process_rss_mb(); // Parses procfs
        
        if current_rss_mb > limit_mb * 0.90 {
            event_bus.broadcast(SystemEvent::MemoryPressure(Level::Critical));
        } else if current_rss_mb > limit_mb * 0.75 {
            event_bus.broadcast(SystemEvent::MemoryPressure(Level::High));
        } else {
            event_bus.broadcast(SystemEvent::MemoryPressure(Level::Normal));
        }
    }
}
```

## 2. Component Subscriptions
Each memory-heavy subsystem subscribes to this bus.
```rust
// Inside the Wasm Pool manager:
if let SystemEvent::MemoryPressure(Level::High) = event {
    self.pool.shrink_to_fit(); // Drop idle pre-warmed instances
}
```