# Design: Benchmark Harness & Documentation

## 1. README Sync
- Move Phases 1-3 to "Completed / Architecture Locked".
- Add Phase 4 ("Grid Computing & TEE") to the roadmap.
- **Quick Start Fix:**
  ```bash
  # 1. Generate the config via Tachyon CLI
  tachyon-cli generate-manifest > integrity.lock
  
  # 2. Start the Mesh
  tachyon-host --config integrity.lock
  ```

## 2. The `bench/` Directory Structure
```text
bench/
├── setup-k3d.sh         # Spins up an isolated local K8s cluster
├── workloads/           # Standard gRPC/HTTP echo payloads
├── mesh-configs/
│   ├── istio-ambient/
│   ├── linkerd/
│   └── tachyon-mesh/
├── run-fortio.sh        # Executes load testing (e.g., 10k QPS)
└── generate-report.py   # Parses JSON results into comparative Markdown charts
```

## 3. Targeted Metrics
The benchmark must explicitly track and graph:
- **Data Plane Latency:** p50, p90, p99, and p99.9 at 1,000 and 10,000 QPS.
- **Control Plane Overhead:** Memory (RSS) consumed per node.
- **Cold Start (Serverless):** Time taken to boot an AI Inference Wasm instance vs a standard Docker container.