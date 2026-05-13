# Design: k8s-operator-e2e

## Overview

Adds a dedicated GitHub Actions workflow that tests the operator path — deploying Tachyon-Mesh from a locally-built Docker image into a k3d cluster and verifying the pod is ready and the admin API responds.

## Task 1 — `.github/workflows/e2e-k8s.yml`

Runs on pushes to `main` that touch `manifests/**`, `core-host/**`, or `Dockerfile`, and on all PRs touching `manifests/**` or `Dockerfile`. Steps:

1. **Build core-host release binary** — `cargo build --release --bin core-host` with Rust stable and dependency cache.
2. **Build local Docker image** — `docker build -t tachyon-mesh:local .`
3. **Spin up k3d cluster** — using `AbsaOSS/k3d-action@v2` (consistent with other CI patterns).
4. **Import local image** — `k3d image import tachyon-mesh:local -c tachyon-test`.
5. **Patch manifest** — `sed` replaces `ghcr.io/astorise/tachyon-mesh:latest` with `tachyon-mesh:local` and `imagePullPolicy: Always` with `Never`.
6. **Apply manifest** — `kubectl apply -f /tmp/deploy-local.yaml`.
7. **Wait for readiness** — `kubectl wait --for=condition=ready pod -l app=tachyon-host --timeout=120s` (using the correct label from `deploy.yaml`).
8. **Healthcheck** — `kubectl port-forward svc/tachyon-service 8080:8080` + `curl -f --retry 5 http://localhost:8080/admin/status`.

## Task 2 — Manifest Verification

`manifests/deploy.yaml` already carries a `readinessProbe` (TCP socket on port 8080) that satisfies `kubectl wait`. The label `app: tachyon-host` matches the workflow's `-l app=tachyon-host` selector. A `livenessProbe` was added in the previous change. No further changes needed.

## Task 3 — Local Image Pipeline

Implemented in the main workflow step: `docker build -t tachyon-mesh:local .` followed by `k3d image import`. The `sed` patch ensures the manifest uses the locally-built image with `imagePullPolicy: Never`.

## Task 4 — CI integration

The existing `k3d-integration` workflow (`integration.yml`) was updated to also sed-patch `deploy.yaml` before applying, since that manifest now references `ghcr.io/astorise/tachyon-mesh:latest` (changed by `operator-zero-build-deployment`). This prevents the integration test from attempting to pull from GHCR.

## CI bug fixes in this commit

- Added `systems/system-faas-openapi` to the workspace `Cargo.toml` — `build-guest-artifacts.sh` references `-p system-faas-openapi` and `cargo` was emitting *"package ID specification did not match any packages"*.
- Updated `integration.yml` deploy step to sed-patch the image reference before `kubectl apply`.
