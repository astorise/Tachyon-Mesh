# Design: Large Model Broker

## Separation From the Asset Registry
- Small `.wasm` assets remain in the embedded registry.
- Large model binaries are written directly to disk under `tachyon_data/models`.
- In-flight multipart uploads are staged under `tachyon_data/model-uploads`.

## Multipart Protocol
1. `POST /admin/models/init` reserves an upload and creates an empty staging file.
2. `PUT /admin/models/upload/:upload_id?part=N` appends the raw chunk directly to disk.
3. `POST /admin/models/commit/:upload_id` re-hashes the finalized file and moves it into `tachyon_data/models/<sha256>.gguf`.

## Client and UI
- The desktop client computes the SHA-256 hash first, then streams the file in 5 MiB chunks.
- The Tauri command wraps the client and emits `upload_progress`.
- The UI binds the emitted percentage to a GSAP-driven progress bar.
