# 🌀 Tachyon-Mesh

**Distributed AI Inference Service Mesh & High-Performance WASM Runtime**

Tachyon is a next-generation, ultra-lightweight Service Mesh written in Rust. It is designed to orchestrate AI workloads and FaaS (Function as a Service) modules in a distributed, secure, and highly performant manner, with specific optimizations for local compute clusters (NVIDIA RTX, Talos OS, Kubernetes).

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Build Status](https://github.com/astorise/tachyon-mesh/actions/workflows/ci.yml/badge.svg)](https://github.com/astorise/tachyon-mesh/actions)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-blue.svg)](https://www.rust-lang.org)
[![WASM](https://img.shields.io/badge/wasm-wasip2-purple.svg)](https://webassembly.org/)

---

## ✨ Key Features

* **⚡ Native WASM Runtime**: Execute polyglot functions (Rust, Go, JS, Python) via WebAssembly (WASIp2) with near-zero cold start latency.
* **🧠 AI-Centric Routing**: "VRAM-aware" intelligent routing capable of allocating AI models to the most optimal GPUs in real-time.
* **🔐 Zero-Trust Security**: Native integration with **IOTA Stronghold** for secret storage, MFA step-up sessions, and PAT (Personal Access Token) authentication.
* **📦 KV-Partition V2**: Distributed and partitioned Key-Value storage for ultra-fast state sharing between functions.
* **📊 Tachyon-UI**: Modern administration console built with Tauri (Vanilla Web Components + Tailwind v4) for visual management of topology and IAM.
* **🤖 MCP Ready**: Built-in Model Context Protocol (MCP) server enabling AI agents (Claude Desktop, Cursor) to autonomously observe and manage your infrastructure.

---

## 🏗 Architecture

Tachyon is built on three core layers:

1. **Core-Host**: The central orchestrator managing the WASM guest lifecycle, L4/L7 networking (HTTP/3, QUIC), and telemetry.
2. **Tachyon-UI**: The secure desktop interface for operators and administrators.
3. **Tachyon-MCP**: The bridge for agentic Artificial Intelligence integration.

---

## 🚀 Quick Start

### Path A — Operators (Zero-Build)

Run Tachyon-Mesh in under a minute without a Rust toolchain.

**Local binary (Linux / macOS):**
```bash
curl -fsSL https://raw.githubusercontent.com/astorise/tachyon-mesh/main/scripts/get-tachyon.sh | bash
./core-host
```

Optional: pin a version with `--version v1.2.3` or choose a directory with `--dir /usr/local/bin`.

**Local binary (Windows — PowerShell):**
```powershell
irm https://raw.githubusercontent.com/astorise/tachyon-mesh/main/scripts/get-tachyon.ps1 | iex
.\core-host.exe
```

Optional: `-Version v1.2.3` or `-Dir C:\Tools\tachyon`.

**Kubernetes (single node or homelab):**
```bash
kubectl apply -f https://raw.githubusercontent.com/astorise/tachyon-mesh/main/manifests/deploy.yaml
```

---

### Path B — Contributors (Build from Source)

Modify the core engine, UI, or FaaS guests.

**Linux / macOS** (requires Rust + Node.js):
```bash
git clone https://github.com/astorise/tachyon-mesh.git
cd tachyon-mesh
./scripts/setup.sh
```

**Windows (PowerShell)**:
```powershell
git clone https://github.com/astorise/tachyon-mesh.git
cd tachyon-mesh
.\scripts\setup.ps1
```

The setup script verifies prerequisites, installs WASM targets, builds all binaries and FaaS guests, installs UI dependencies, runs cross-layer validation, and prints the exact commands and MCP config snippet you need.

After setup:
```bash
# Terminal 1 — start the mesh
./target/release/core-host

# Terminal 2 — launch the operator UI
cd tachyon-ui && npm run tauri dev
```

---

## 🧩 Building FaaS Guests — WIT Contracts via OCI

Tachyon publishes its WebAssembly Interface Types (WIT) contracts as OCI artifacts to GitHub Container Registry. Guest developers do **not** need to copy `.wit` files locally.

Add the following to your component's `Cargo.toml` (requires [`cargo-component`](https://github.com/bytecodealliance/cargo-component)):

```toml
[package.metadata.component.dependencies]
"tachyon:mesh" = { registry = "oci", package = "ghcr.io/astorise/tachyon-mesh-wit", version = "1.0.0" }
```

`cargo-component build` will automatically fetch and resolve the WIT interfaces during compilation. Replace `1.0.0` with the release tag you are targeting.

To see all available versions:
```bash
wkg list ghcr.io/astorise/tachyon-mesh-wit
```

---

## 🛠️ IDE Integration & Schema Validation

While `core-host` is running, it serves live JSON Schema documents for its configuration files — enabling real-time validation and autocompletion in VS Code, JetBrains, and Neovim without copying schema files.

**VS Code quick setup** (`.vscode/settings.json`):
```json
{
  "json.schemas": [
    { "fileMatch": ["**/integrity.lock"], "url": "http://127.0.0.1:8080/admin/schema/integrity-lock" }
  ]
}
```

**YAML modeline:**
```yaml
# yaml-language-server: $schema=http://127.0.0.1:8080/admin/schema/manifest
```

See **[docs/ide-integration.md](docs/ide-integration.md)** for JetBrains, Neovim, HTTP Client, and offline/air-gapped setups.

---

## 🖥️ GPU / Homelab Kubernetes Deployment

For hardware-accelerated clusters (NVIDIA GPU nodes, model-cache PVC, Prometheus ServiceMonitor):

```bash
kubectl apply -f manifests/deploy-gpu-homelab.yaml
```

See **[manifests/deploy-gpu-homelab.yaml](manifests/deploy-gpu-homelab.yaml)** for the full manifest including `nodeSelector`, GPU resource limits, model-cache PVC, and Prometheus ServiceMonitor placeholder.

---

## 🔧 Troubleshooting

Encountering build errors, port conflicts, missing GPU detection, or `-32001 Cluster Unreachable` from the MCP server?

See **[TROUBLESHOOTING.md](TROUBLESHOOTING.md)** for the full guide covering 15 common failure modes across build, runtime, UI, MCP, and Kubernetes/GPU domains.

---

## 🤖 LLM Agent Integration (MCP)

Tachyon exposes its cluster capabilities to AI agents via the **Model Context Protocol**. This allows your development tools to deploy functions, validate manifests, or check cluster telemetry using natural language.

### Claude Desktop Configuration
Add the following to your `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "tachyon-mesh": {
      "command": "cargo",
      "args": ["run", "--bin", "tachyon-mcp"],
      "env": {
        "TACHYON_MCP_URL": "http://127.0.0.1:8080",
        "TACHYON_MCP_PAT": "YOUR_PERSONAL_ACCESS_TOKEN"
      }
    }
  }
}
```
*For more details, see [docs/mcp-setup.md](docs/mcp-setup.md).*

---

## 🛠 Function Development (Guests)

Tachyon leverages the **WASM Component Model**. Here is a minimal Rust example:

```rust
use tachyon_sdk::prelude::*;

#[tachyon_function]
fn handle_request(req: Request) -> Response {
    Response::builder()
        .status(200)
        .body("Hello from Tachyon Mesh!")
        .build()
}
```

To build a guest artifact:
```bash
scripts/build-guest-artifacts.sh examples/guest-example
```

---

## 🔒 Security & Governance

* **Identity**: Fine-grained Role-Based Access Control (RBAC) and user group management.
* **Integrity**: Every WASM artifact is cryptographically verified against `integrity.lock` prior to execution.
* **Isolation**: Strict resource sandboxing (CPU, RAM, VRAM allocation).
* **MFA**: Critical control plane actions (e.g., Apply Manifest, Delete) require explicit MFA approval via the UI.

---

## 🗺 Roadmap

- [x] VRAM-aware routing and multi-GPU optimization.
- [x] Distributed KV-Store (Partitioning V2).
- [x] Tauri Interface (Phase 3: Routing Dashboards complete).
- [ ] **Upcoming**: GPU pressure-based auto-scaling (KEDA integration).
- [ ] **Upcoming**: Native air-gapped asset registry.

---

## 🤝 Contributing

Contributions are welcome! Please refer to `CONTRIBUTING.md` (coming soon) and use the `openspec/` directory format for any architectural change proposals.

---

## 📄 License

Distributed under the MIT License. See `LICENSE` for more information.