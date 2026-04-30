# Design: Asynchronous Fire-and-Forget Shadowing

## 1. Schema Update (`integrity.lock`)
```json
"routes": {
  "/api/ai/generate": {
    "target": "llama-3-stable",
    "shadow_target": "llama-3-experimental-lora"
  }
}
```

## 2. Core-Host Dispatcher Modification
The `core-host` remains lightweight. It only needs to clone the payload and emit an event.
```rust
// Inside core-host/src/network/router.rs
let response = execute_primary(&route.target, &payload).await?;

// Fire & Forget (does not block the response to the client)
if let Some(shadow_target) = &route.shadow_target {
    let shadow_event = ShadowEvent {
        payload: payload.clone(),
        shadow_target: shadow_target.clone(),
        primary_status: response.status(),
        primary_hash: hash(&response.body),
    };
    // Send to the optional System FaaS via IPC
    let _ = ipc_client.send_async("system-faas-shadow-proxy", shadow_event);
}

return Ok(response);
```

## 3. The Shadow Proxy FaaS (`systems/system-faas-shadow-proxy`)
This Wasm module runs independently in the background pool.
- It receives the `ShadowEvent`.
- It makes an internal loopback IPC call to execute the `shadow_target`.
- It hashes the shadow response.
- If `shadow_hash != primary_hash`, it emits a structural diff metric to the telemetry bus.