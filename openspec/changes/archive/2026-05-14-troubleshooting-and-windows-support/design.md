# Design: troubleshooting-and-windows-support

## Task 1 — `TROUBLESHOOTING.md`

Created in the repository root. Structured into 7 domains with 15 failure modes:

| Domain | Failure modes |
|---|---|
| Build & Compilation | wasm target missing, MSVC tools, NASM/CMake |
| Core-Host Runtime | port conflict, integrity.lock signature mismatch, ONNX Runtime |
| Tachyon-UI (Tauri) | WebKitGTK headers, WiX Toolset (Windows) |
| Tachyon-MCP | -32001, -32002, degraded manifest schema |
| Kubernetes & GPU | VRAM scheduling, GPU not detected |
| General diagnostics | structured logs, state reset, cross-layer validation |

`README.md` gains a **Troubleshooting** section immediately after the After Setup block, linking to `TROUBLESHOOTING.md` with a one-line description.

## Task 2 — Windows Release Pipeline

`release.yml` `publish-server-binaries` matrix gains a new entry:
```yaml
- os: windows-latest
  target: x86_64-pc-windows-msvc
  os_name: windows
  arch: x86_64
```

The "Package tarball" step is renamed "Package archive" and updated to branch on `OS`:
- **Windows**: produces `tachyon-mesh-VERSION-windows-x86_64.zip` using `7z a` (pre-installed on GitHub-hosted Windows runners) with `.exe` suffixes.
- **Linux/macOS**: unchanged `tar.gz` behaviour.

The `TARBALL` env variable is reused for both paths so the upload step requires no change.

## Task 3 — MCP Schema Warning

- Added `tracing = "0.1"` to `tachyon-mcp/Cargo.toml`.
- Replaced `let _ = MANIFEST_SCHEMA.set(schema)` with a `match` that emits `warn!(...)` on schema fetch failure, describing the degradation.
- In the `"tools/list"` handler: the response is now built into a `mut tools_result` variable. If `MANIFEST_SCHEMA.get().is_none()`, a `data.warnings` array is appended before returning, advising the agent that manifest field guidance is unavailable.
