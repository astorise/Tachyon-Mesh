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

## CI regression and cold-start measurements

The separate `Bench Regression` workflow measures the FaaS chain at 200 QPS,
16 connections and 30 seconds by default, and repeats a single cold request
after restarting the host. It does not run the mesh comparison below. Its
weekly run takes five cold-start samples per variant as a smoke test. The
report shows a cold-start p99 only when that variant has at least 100 samples;
even 100 is a low sample count for a stable p99 estimate.

For an exploratory run with 100 instance-pre samples on a chosen branch:

```sh
gh workflow run bench-regression.yml --ref BRANCH -f cold_start_samples=100
```

The workflow uploads the raw Fortio JSON, cold-start JSONL, report and
`profile.json` metadata as a run artifact. It fails when required data is
missing, requests fail, or configured thresholds are exceeded. The forced
transport comparison remains at five cold starts and therefore has no p99.
If the instance-pre p99 threshold is enabled, the scheduled run must also be
configured to collect at least 100 samples; otherwise it fails closed.

GitHub-hosted runners are shared and do not establish the isolated hardware or
separate load generator required for a public comparison. Repeat measurements
on a documented representative environment before selecting a threshold or
making a numerical performance promise. The null latency thresholds are not
performance guarantees.

## Standard Profile

Public baseline runs should use a dedicated AWS `c6i.xlarge` or equivalent x86_64 host with no colocated workloads, k3d backed by Docker, and the default script parameters:

- 60 second Fortio duration
- 64 concurrent connections
- 1,000 QPS and 10,000 QPS target rates
- Echo workload replicas pinned to two pods per mesh namespace

If a run uses different hardware or parameters, record that in `bench/results/profile.md` before publishing the generated report.
