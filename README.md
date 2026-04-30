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
Create or refresh `integrity.lock` from the Tachyon Studio desktop configurator, then keep the signed manifest at the repository root. For shell-based startup, validate that the manifest exists and point the host at it explicitly:
```bash
test -s integrity.lock
export TACHYON_INTEGRITY_MANIFEST="$PWD/integrity.lock"
```

### 4. Run the Host
The host validates the sealed `integrity.lock` manifest and binds to your local port, ready to serve requests directly to the WASM modules.
```bash
cargo run -p core-host --release
```

### 5. Optional Autoscaling System FaaS
Build the autoscaling guests when you want queue-depth metrics or autonomous legacy scaling:
```bash
cargo build -p system-faas-keda --target wasm32-wasip2 --release
cargo build -p system-faas-k8s-scaler --target wasm32-wasip2 --release
```

Seal `/metrics/scaling` to expose Prometheus queue depth for `/api/guest-call-legacy`:
```bash
test -s integrity.lock
```

Seal `/system/k8s-scaler` to enable the five-second background autoscaler. For local validation against a mock API server, point the host at the mock base URL before starting it:
```bash
export TACHYON_MOCK_K8S_URL="http://127.0.0.1:18080"
cargo run -p core-host --release
```

### 6. Optional WASI-NN Inference Guest
Build the legacy `guest-ai` module when you want ONNX inference through WASI-NN:
```bash
cargo build -p guest-ai --target wasm32-wasip1 --release
```

Seal the AI route and mount a read-only model directory into the guest. The guest expects ONNX files under `/models` and defaults to `/models/model.onnx`:
```bash
test -s integrity.lock
```

Run the host with the optional feature so it exposes the `wasi_ephemeral_nn` imports:
```bash
cargo run -p core-host --features ai-inference --release
```

Send a JSON request with the input tensor shape, input values, and an output buffer size:
```bash
curl --request POST http://127.0.0.1:8080/api/guest-ai \
  --header "content-type: application/json" \
  --data "{\"model\":\"model.onnx\",\"shape\":[1,4],\"values\":[1.0,2.0,3.0,4.0],\"output_len\":4}"
```

`ai-inference` is intentionally optional because the host machine must provide ONNX Runtime dynamic libraries for the selected execution provider.

## Performance & Benchmarks

The reproducible benchmark harness lives in [`bench/`](bench/README.md). It provisions a clean k3d cluster, deploys a neutral echo workload behind Istio Ambient, Linkerd, and Tachyon Mesh adapters, runs Fortio load tests, captures Kubernetes resource snapshots, and renders Markdown tables from raw JSON output.

No latency or memory claim should be updated in this README without committing the raw benchmark output under `bench/results/` and the generated report that cites the cluster profile used for the run.

## 🗺️ Roadmap

- [x] **Phase 1:** Core Wasmtime Orchestrator & Axum HTTP Routing.
- [x] **Phase 2:** FaaS-native observability and privileged system FaaS routes.
- [x] **Phase 3:** Ed25519 build-time cryptographic validation and sealed manifest tooling.
- [x] **Phase 4:** Compile-time service mesh, resiliency, rate limits, and traffic policy.
- [ ] **Phase 5:** Grid computing, TEE-backed route execution, and hardware-aware admission.
- [ ] **Phase 6:** Hybrid Mesh: ultra-light Rust sidecar for external OCI containers.

## 🤝 Contributing
We believe in a leaner, faster, and more secure serverless future. PRs are welcome!

### WIT and SDK versioning
Every public WIT package under `wit/` is part of the SDK compatibility contract and must declare an explicit SemVer package version on the first line, for example `package tachyon:mesh@1.0.0;`.

Backward-compatible additions keep the current major version and may bump minor or patch versions. Breaking changes, including removed functions, renamed fields, changed record shapes, or incompatible type changes, require a major version bump. Pull requests that modify WIT files run `.github/workflows/wit-compat.yml`, which validates the package declarations and checks the changed interfaces with `wasm-tools` against the target branch.

## Testing Standards

Core host changes should keep business logic behind small module boundaries and include focused tests in the same change. New routing, manifest parsing, storage, identity, or IPC behavior should include unit tests; parser and normalization code should prefer property-based tests with `proptest`.

Integration scenarios live under `core-host/tests/`. Tests that need signed manifests, compiled guest artifacts, HTTP/3 sockets, or host-level runtime services should be marked `#[ignore]` and document their prerequisites. CI generates an LCOV report for `core-host` with `cargo llvm-cov` so coverage can be uploaded to Codecov or another coverage service.
