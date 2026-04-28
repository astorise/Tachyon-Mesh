# Design: Hybrid Engine and TEE Dispatcher

## 1. Schema Update (`core-host/src/main.rs`)
Add the hardware security flag to the Wasm module configuration schema.

```rust
pub struct FaaSConfig {
    pub module_name: String,
    // ...
    #[serde(default)]
    pub requires_tee: bool, // Defaults to false
}
```

## 2. Dispatcher Logic (`core-host/src/store/dispatcher.rs`)
The Wasm invocation router must split the execution path based on the manifest flag.

```rust
if config.requires_tee {
    // 1. Slow path: High Security
    // Pass the payload via secure IPC to the system-faas-tee-runtime 
    // or invoke the enclave loader (e.g., Enarx keep).
    tracing::info!("Delegating execution to hardware enclave (TEE)");
    execute_in_enclave(&module_id, &payload).await?
} else {
    // 2. Fast path: Standard pooled execution
    // Fetch from InstancePre cache and execute in standard RAM
    execute_in_standard_pool(&module_id, &payload).await?
}
```

## 3. The TEE System FaaS (`systems/system-faas-tee-runtime`)
Instead of bloating `core-host` with complex Intel SGX/AMD SEV SDKs, we create a dedicated optional `system-faas` compiled with TEE capabilities.
- This module is responsible for setting up the enclave, loading the user's Wasm payload inside it, and passing the data in and out via encrypted memory buffers.
- *Note: This requires specific hardware compatibility checks at node boot.*