# Specifications: Embedded Registry

## 1. Registry Storage Engine
The asset registry will leverage the existing internal key-value store (e.g., `redb` configured in `turboquant-kv`).
- Keys will be the SHA-256 hash of the binary (e.g., `sha256:a1b2c3d4...`).
- Values will be the raw binary blob (WASM or OCI image layers).

## 2. Push API Contract
The `core-host` must expose an administrative endpoint to receive blobs.
- Because WASM files can be large (megabytes), the API must support streaming or chunked uploads to avoid exhausting memory on the receiving edge node.
- Upon completion, the server calculates the SHA-256 hash, verifies it against the client's provided checksum, stores it, and returns the hash as the exact URI to be used in future manifest deployments.

## 3. Wasmtime Loader Refactoring
In `core-host`, the function responsible for loading a WASM module into the engine must be refactored.
- If a deployment manifest requests an image like `tachyon://sha256:a1b2...`, the host will bypass external HTTP/HTTPS clients entirely and read the bytes directly from the local KV store.