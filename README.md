# ⚡ Tachyon Mesh

![Rust](https://img.shields.io/badge/Rust-1.80%2B-orange.svg)
![WebAssembly](https://img.shields.io/badge/WebAssembly-WASI-blue.svg)
![License](https://img.shields.io/badge/License-MIT-green.svg)

**Tachyon Mesh** is an ultra-lightweight, sidecar-less Serverless/FaaS platform and Service Mesh, built entirely in Rust and WebAssembly.

It completely rethinks the Cloud Native stack by replacing the massive CPU and memory overhead of Kubernetes, Knative, and Istio/Envoy with **in-process WASM sandboxing**, **cryptographic build-time validation**, and a **compile-time service mesh**. It is designed to run ultra-dense WASM FaaS workloads today, with a clear path to supporting legacy containers via an ultra-light Rust sidecar tomorrow.

## 🛑 The Problem

Modern architectures (like Knative + Linkerd/Istio) suffer from the **"Sidecar Tax"**. For every lightweight function you deploy, you also deploy an Envoy proxy and a Queue-Proxy container. This results in:
- **Massive Memory Overhead:** 100MB+ per pod just for infrastructure.
- **Slow Cold Starts:** Booting OS-level containers and initializing TCP network meshes takes time.
- **Configuration Drift:** Runtime YAML mutations lead to untraceable security breaches.

## 🚀 The Tachyon Solution

Tachyon Mesh drops the heavy containers and bloated network proxies. It runs a single highly-optimized Rust Host that orchestrates multiple FaaS functions compiled as WebAssembly components (`wasm32-wasip2`) or legacy WASI modules (`wasm32-wasip1`) within the same OS process.

### Key Innovations
- 🪶 **Zero-Overhead FaaS:** Network routing and observability are handled natively by the Rust Host. Component guests use a typed WIT boundary, while legacy guests still run through the existing WASI pipeline.
- 🔒 **Cryptographic Integrity:** Configuration is signed locally via an Ed25519 key pair. The Rust Host validates its own runtime configuration signature at startup. If tampered with, it panics.
- 🧬 **Compile-Time Service Mesh:** Features like Chaos Testing and A/B Canary releases are injected via Rust `features` at compile-time. If you don't need them, they add *zero bytes* and *zero CPU cycles* to your binary.
- 🔭 **Macro-Based Observability:** A simple `#[faas_handler]` macro instruments your WASM functions. Logs are intercepted and exported by the Host. No heavy OpenTelemetry SDKs in your FaaS binaries.
- 🔌 **Hybrid Future-Proofing:** Designed to seamlessly integrate standard Docker containers using an ultra-light, purpose-built Rust sidecar sharing the same compile-time mesh philosophy.

## 🏗️ Architecture overview

```text
[ Incoming HTTP Request ]
       │
       ▼
┌────────────────────────────────────────────────────────┐
│  Tachyon Core Host (Rust / Axum)                       │
│                                                        │
│  ├── Compile-Time Middleware (Chaos, Canary, Retry)    │
│  ├── Telemetry Interceptor (OTLP Exporter)             │
│  │                                                     │
│  └── Wasmtime Engine (Strict Memory & Fuel Quotas)     │
│         │                                              │
│         ▼ (Typed WIT / Legacy WASI Fallback)          │
│      ┌─────────────────────┐   ┌─────────────────────┐ │
│      │ guest_payment.wasm  │   │ guest_auth.wasm     │ │
│      └─────────────────────┘   └─────────────────────┘ │
└────────────────────────────────────────────────────────┘
       │
       ▼ (Future: Ultra-light TCP/gRPC sidecar routing)
┌────────────────────────────────────────────────────────┐
│  Legacy Container (Go/NodeJS) + Tachyon Micro-Sidecar  │
└────────────────────────────────────────────────────────┘
```

## 🛠️ Quick Start

### 1. Prerequisites
- Rust toolchain (`cargo`, `rustup`)
- WebAssembly targets: `rustup target add wasm32-wasip2 wasm32-wasip1`

### 2. Build the Guest Function
Write your FaaS logic using our lightweight SDK:
```rust
use faas_sdk::faas_handler;

#[faas_handler]
pub fn process_data() {
    tracing::info!("Function executed without network overhead!");
    println!("Hello from Tachyon Mesh");
}
```
Compile the Rust reference guest as a WebAssembly component:
```bash
cargo build -p guest-example --target wasm32-wasip2 --release
```

### 3. Seal the Runtime Configuration
Generate the signed `integrity.lock` manifest that `core-host` embeds and validates at startup:
```bash
cargo run -p tachyon-cli -- generate --route /api/guest-example --system-route /metrics --memory 64
```

### 4. Run the Host
The host validates the sealed `integrity.lock` manifest and binds to your local port, ready to serve requests directly to the WASM modules.
```bash
cargo run -p core-host --release
```

## 🗺️ Roadmap

- [ ] **Phase 1:** Core Wasmtime Orchestrator & Axum HTTP Routing (In-memory pipes).
- [ ] **Phase 2:** FaaS-Native Observability via `#[faas_handler]` macro.
- [ ] **Phase 3:** Ed25519 Build-Time Cryptographic Validation (`tachyon-cli`).
- [ ] **Phase 4:** Compile-Time Service Mesh (Traffic shifting & autonomous A/B testing).
- [ ] **Phase 5:** Rich Tauri GitOps Desktop Client for visual configuration generation.
- [ ] **Phase 6:** Hybrid Mesh: Ultra-light Rust sidecar for external OCI containers.

## 🤝 Contributing
We believe in a leaner, faster, and more secure serverless future. PRs are welcome!
