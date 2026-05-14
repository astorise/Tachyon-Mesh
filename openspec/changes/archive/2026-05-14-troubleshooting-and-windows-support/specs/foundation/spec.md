# Technical Specification: Documentation

## 1. `TROUBLESHOOTING.md` Creation
Create a new file in the root directory. It must be structured by domain.

**Required Sections:**
- **Build & Compilation:**
  - "Target `wasm32-wasip2` not found" -> `rustup target add wasm32-wasip2`
  - "Missing MSVC/C++ Build Tools (Windows)"
- **Core-Host Runtime:**
  - "Address already in use (os error 98)" -> Port 8080 conflict resolution.
  - "Invalid signature in `integrity.lock`" -> Running `scripts/build-guest-artifacts.sh` to regenerate.
  - "ONNX Runtime missing" -> AI inference dynamic library linking.
- **Tachyon-UI (Tauri):**
  - "WebKitGTK development headers missing (Linux)" -> `apt install libwebkit2gtk-4.1-dev`
- **Tachyon-MCP:**
  - "MCP Agent returns -32001 Cluster Unreachable" -> Verify host is running and PAT is valid.
- **Kubernetes / GPU:**
  - "Pods pending / Insufficient VRAM" -> Talos/K3s nodeSelector config.

## 2. `README.md` Update
Add a dedicated "Troubleshooting" block right after the Quickstart section pointing to this new file.