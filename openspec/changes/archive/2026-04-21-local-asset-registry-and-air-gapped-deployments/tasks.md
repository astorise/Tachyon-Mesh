# Tasks: Change 070 Implementation

**Agent Instruction:** Implement the Air-Gapped Local Registry flow.

- [x] Add an embedded asset-registry bucket to the persistent host store and implement `save_asset` / `load_asset`.
- [x] Add an admin-protected asset upload route in `core-host` that verifies the SHA-256 checksum and returns a `tachyon://sha256:...` URI.
- [x] Add `push_asset(file_path)` and `push_asset_bytes(...)` in `tachyon-client` so desktop flows can upload compiled `.wasm` assets.
- [x] Add a Tauri `push_asset` command plus a deployment card in the UI with a `.wasm` picker and upload trigger.
- [x] Teach guest-module resolution to materialize `tachyon://sha256:...` assets from the embedded registry before Wasmtime loads them.
