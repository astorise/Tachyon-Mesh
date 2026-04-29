# Design: SmolVM Runner Integration

## 1. Schema Update (`core-host/src/main.rs`)
Extend the FaaS configuration to support the MicroVM runtime.

```rust
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum FaaSRuntime {
    Wasm { source: String },
    MicroVM { image: String, vcpus: u8, memory_mb: u32 },
}

pub struct FaaSConfig {
    pub module_name: String,
    pub runtime: FaaSRuntime,
    // ...
}
```

## 2. Dispatcher Branching (`core-host/src/store/dispatcher.rs`)
Update the routing logic to branch based on the runtime type:

```rust
match &config.runtime {
    FaaSRuntime::Wasm { .. } => {
        // Standard sub-millisecond Wasm execution
        execute_in_wasm_pool(&module_id, &payload).await
    },
    FaaSRuntime::MicroVM { .. } => {
        // Delegate to the SmolVM system runner
        tracing::info!("Delegating execution to MicroVM Runner");
        ipc_client.send_to("system-faas-microvm-runner", module_id, payload).await
    }
}
```

## 3. The MicroVM Runner FaaS (`systems/system-faas-microvm-runner`)
This module requires host-level access to `/dev/kvm` (Linux).
- **Boot Logic:** Uses the `smolvm` Rust crate to load the kernel and rootfs from the `.smolmachine` file.
- **Proxy Layer:** Creates a virtual socket (vsock) between the host and the guest OS. When a Tachyon request arrives, the runner forwards the JSON payload over the vsock to a tiny listening daemon inside the MicroVM.
- **Lifecycle:** Terminates the MicroVM process once the response is returned (or keeps it warm based on pooling configurations).