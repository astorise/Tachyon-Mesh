## Why

The current test suite proves host behavior locally, but Tachyon Mesh is intended to run as a lean Kubernetes workload. We need an automated end-to-end check that verifies the container image, the cluster deployment, the integrity-gated startup path, and HTTP routing inside a real ephemeral cluster.

## What Changes

- Add a new `k3d-integration-test` capability covering container packaging, Kubernetes manifests, and GitHub Actions orchestration.
- Define a multi-stage container build that produces a minimal runtime image containing a statically linked `core-host`, a compiled guest module, and the assets needed for startup.
- Define manifests that deploy the container to a disposable k3d cluster without relying on an external image registry.
- Define an integration workflow that builds the image, creates a cluster, deploys the host, and verifies the `/api/guest-example` endpoint through an HTTP assertion.

## Capabilities

### New Capabilities

- `k3d-integration-test`: End-to-end validation of the packaged Tachyon Mesh host inside an ephemeral k3d Kubernetes cluster.

### Modified Capabilities

- None.

## Impact

- Adds a root `Dockerfile`, Kubernetes manifests, and `.github/workflows/integration.yml`.
- Requires CI support for Docker, `x86_64-unknown-linux-musl`, `wasm32-wasip1`, `kubectl`, and `k3d`.
- Verifies that the packaged host reaches `Running` and serves HTTP traffic after cluster deployment.
