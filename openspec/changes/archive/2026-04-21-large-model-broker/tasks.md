# Tasks: Change 071 Implementation

**Agent Instruction:** Implement the Direct-to-Disk streaming protocol for AI models. Do not use memory buffering for the upload streams.

- [x] Add a dedicated direct-to-disk model broker in `core-host` with `/admin/models/init`, `/admin/models/upload/:upload_id`, and `/admin/models/commit/:upload_id`.
- [x] Stream incoming model chunks into staging files under `tachyon_data/model-uploads` and finalize them into `tachyon_data/models`.
- [x] Add `push_large_model` plus a chunked upload loop in `tachyon-client` that reads files in 5 MiB buffers.
- [x] Add a Tauri `push_large_model` command that emits `upload_progress` while the Rust client streams the file.
- [x] Add a model upload section in the UI with a progress bar driven by the `upload_progress` event stream.
