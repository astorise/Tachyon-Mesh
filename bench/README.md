# Tachyon Mesh Benchmark Harness

This directory contains the reproducible public benchmark workflow for comparing Tachyon Mesh with common service-mesh data planes. The harness favors raw artifacts over marketing numbers: every published table must be generated from committed Fortio JSON and Kubernetes resource snapshots.

## Prerequisites

- Docker
- k3d
- kubectl
- fortio
- Python 3.10+
- Optional: Istio and Linkerd CLIs if you want to install those meshes automatically

## Workflow

```bash
bench/setup-k3d.sh
kubectl apply -f bench/workloads/echo.yaml
bench/run-fortio.sh
bench/collect-resources.sh
python bench/generate-report.py
```

The scripts write raw outputs to `bench/results/raw/` and generate `bench/results/report.md`.

## Standard Profile

Public baseline runs should use a dedicated AWS `c6i.xlarge` or equivalent x86_64 host with no colocated workloads, k3d backed by Docker, and the default script parameters:

- 60 second Fortio duration
- 64 concurrent connections
- 1,000 QPS and 10,000 QPS target rates
- Echo workload replicas pinned to two pods per mesh namespace

If a run uses different hardware or parameters, record that in `bench/results/profile.md` before publishing the generated report.
