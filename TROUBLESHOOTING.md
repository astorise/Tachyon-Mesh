# Tachyon Mesh — Troubleshooting Guide

Common failure modes, their causes, and the fastest resolution path.

---

## Build & Compilation

### Target `wasm32-wasip2` not found

```
error[E0463]: can't find crate for `core`
  = note: the `wasm32-wasip2` target may not be installed
```

**Fix:**
```bash
rustup target add wasm32-wasip1 wasm32-wasip2
```

---

### Missing MSVC / C++ Build Tools (Windows)

```
error: linker `link.exe` not found
```

**Fix:** Install the **Build Tools for Visual Studio 2022** (free).
Download from <https://visualstudio.microsoft.com/downloads/#build-tools-for-visual-studio-2022>.
Select the **"C++ build tools"** workload during install.

Alternatively, use the `x86_64-pc-windows-gnu` toolchain with the MinGW-w64 linker.

---

### NASM / CMake not found (`aws-lc-sys` build fails)

```
thread 'main' panicked ... "Required build dependency is missing. Halting build."
```

This occurs when building with the `fips` or `ring` feature and the system lacks `nasm` or `cmake`.

**Fix (Ubuntu/Debian):**
```bash
sudo apt-get install -y cmake nasm
```

**Fix (macOS):**
```bash
brew install cmake nasm
```

---

## Core-Host Runtime

### Address already in use (os error 98)

```
Error: failed to bind HTTP listener on 0.0.0.0:8080 ... address already in use
```

**Fix:** Find and stop the conflicting process:
```bash
# Linux/macOS
lsof -i :8080
kill -9 <PID>

# Windows
netstat -ano | findstr :8080
taskkill /PID <PID> /F
```

Or change the bind address in `integrity.lock`:
```json
{ "hostAddress": "0.0.0.0:8081", ... }
```

---

### Invalid signature in `integrity.lock`

```
Error: integrity check failed: signature mismatch
```

The lock file was signed with a different node key, or the WASM artifacts changed after sealing.

**Fix:** Rebuild the guest artifacts and re-seal:
```bash
bash scripts/build-guest-artifacts.sh
# Then restart core-host — it regenerates integrity.lock on first run
```

---

### ONNX Runtime missing (`ai-inference` feature)

```
Error: libonnxruntime.so not found
```

The `ai-inference` feature links dynamically against the ONNX Runtime shared library.

**Fix:**
```bash
# Download from https://github.com/microsoft/onnxruntime/releases
# Place the .so/.dylib/.dll in a directory on LD_LIBRARY_PATH
export LD_LIBRARY_PATH=/opt/onnxruntime/lib:$LD_LIBRARY_PATH
```

Or set `ORT_DYLIB_PATH` to the absolute path of the library before building.

---

## Tachyon-UI (Tauri)

### WebKitGTK development headers missing (Linux)

```
error: Package 'webkit2gtk-4.1' was not found
```

**Fix:**
```bash
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev \
  build-essential \
  libssl-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev
```

---

### Tauri build fails: `WixToolset` not found (Windows)

```
Error Failed to bundle project: "WixToolset not found"
```

**Fix:** Install [WiX Toolset v3](https://wixtoolset.org/releases/) and ensure it is on `PATH`.

---

## Tachyon-MCP

### `-32001 Cluster Unreachable`

```json
{ "error": { "code": -32001, "message": "Cluster unreachable: ..." } }
```

**Checklist:**
1. Verify `core-host` is running: `curl http://127.0.0.1:8080/admin/status`
2. Verify `TACHYON_MCP_URL` matches the host address in `integrity.lock`.
3. Verify `TACHYON_MCP_PAT` is a valid Personal Access Token issued by the node.
4. If using TLS, ensure `TACHYON_MCP_CERT` points to the correct CA bundle.
5. Check firewall rules — port 8080 must be reachable from the MCP process.

---

### `-32002 Rate Limited`

```json
{ "error": { "code": -32002, "data": { "retry_after_ms": 45000 } } }
```

The tool was called more frequently than its per-minute budget allows. Wait `retry_after_ms` before retrying. For `tachyon_canary_split` the limit is 2/min; for deployment tools it is 5/min.

---

### Dynamic manifest schema unavailable (degraded tool definitions)

If `tachyon_dryrun_manifest` does not show detailed field descriptions, the MCP server failed to fetch the schema from `GET /admin/schema/manifest`.

**Fix:** Ensure core-host is reachable and restart the MCP process — it fetches the schema on the first authenticated request.

---

## Kubernetes & GPU

### Pods pending: `Insufficient VRAM` / `Unschedulable`

```
0/1 nodes are available: 1 Insufficient memory.
```

When a FaaS route specifies `vram_mb > 0`, the scheduler rejects nodes without sufficient VRAM headroom.

**Fix (K3s/Talos):** Add a `nodeSelector` in your manifest's `resourcePolicy`:
```json
{
  "resourcePolicy": {
    "vramMb": 4096,
    "gpuAffinity": "cuda:0",
    "admissionStrategy": "mesh_retry"
  }
}
```

Use `mesh_retry` instead of `fail_fast` to allow the scheduler to queue the invocation until a capable node is available.

---

### GPU not detected (`accelerators: ["cpu"]` only)

The hardware status endpoint returns only `"cpu"` even though a GPU is present.

**Fix:** Set the appropriate environment variable before starting `core-host`:
```bash
# NVIDIA
export CUDA_VISIBLE_DEVICES=0

# AMD (ROCm)
export HIP_VISIBLE_DEVICES=0
```

---

## General Diagnostics

### View structured logs

```bash
RUST_LOG=tachyon_mesh=debug ./target/release/core-host
```

### Reset local state

```bash
rm integrity.lock
rm -f ~/.local/share/tachyon-mesh/*.lock   # Linux
```

### Cross-layer validation

```bash
bash scripts/validate_cross_layer.sh
```

---

*Still stuck? Open an issue at <https://github.com/astorise/tachyon-mesh/issues>.*
