# Design: FaaS-Driven TDE Architecture

## 1. Schema Update (`integrity.lock` & `core-host/src/main.rs`)
Update the volume configuration structures to include the `encrypted` flag.

```rust
#[derive(Debug, Deserialize, Serialize)]
pub struct VolumeMount {
    pub name: String,
    pub path: String,
    #[serde(default)]
    pub encrypted: bool, // Defaults to false to prevent overhead
}
```

## 2. The TDE System FaaS (`systems/system-faas-tde`)
Create a new Wasm system module that exposes two primary IPC functions:
- `encrypt_chunk(data: Vec<u8>, nonce: u64) -> Vec<u8>`
- `decrypt_chunk(data: Vec<u8>, nonce: u64) -> Vec<u8>`

*Note: The master key is securely provisioned to this FaaS at startup via the `system-faas-secrets` broker.*

## 3. Host WASI Delegation (`core-host/src/system_storage.rs`)
When initializing the `wasmtime-wasi` context for a user FaaS, the host checks the `encrypted` flag for each mounted directory.

- **If `encrypted == false`:** Bind the standard `Dir::open_ambient_dir`.
- **If `encrypted == true`:** Wrap the directory in a custom virtual filesystem (VFS) implementation. When `write()` or `read()` is called, the VFS pauses the execution, dispatches an IPC request to `system-faas-tde` with the data chunk, and resumes upon receiving the ciphered/deciphered result.