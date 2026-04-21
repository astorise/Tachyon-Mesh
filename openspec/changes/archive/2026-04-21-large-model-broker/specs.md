# Specifications: Large Model Registry

## 1. Storage Isolation
Models MUST NOT be stored in the `redb` KV store. The `system-faas-model-broker` must write files directly to the local filesystem (e.g., `tachyon_data/models/{sha256}.gguf`).

## 2. Multipart API Protocol
The `core-host` will expose the following sequence:
- **POST `/admin/models/init`**:
  - Request: `{ "expected_hash": "sha256:abc...", "size_bytes": 4200000000 }`
  - Response: `{ "upload_id": "uuid-123" }`
- **PUT `/admin/models/upload/{upload_id}?part=1`**:
  - Body: Raw binary bytes (e.g., 5MB chunk).
  - The host opens the file in `append` mode and writes the bytes immediately to disk, discarding them from RAM.
- **POST `/admin/models/commit/{upload_id}`**:
  - Triggers the host to finalize the file, calculate the final SHA-256 hash of the written file, and verify it against the `expected_hash`.

## 3. Client & UI Slicing
The `tachyon-client` must implement `async fn push_large_model(path: &Path, progress_callback: fn(f32))` that reads the file in chunks and handles the sequence above.