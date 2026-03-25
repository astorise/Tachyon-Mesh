## 1. Container Build

- [ ] 1.1 Add a root `Dockerfile` with a builder stage that installs `wasm32-wasip1` and `x86_64-unknown-linux-musl`.
- [ ] 1.2 Compile `guest-example` for `wasm32-wasip1`, generate `integrity.lock`, and build a release `core-host` binary for `x86_64-unknown-linux-musl`.
- [ ] 1.3 Produce a minimal runtime image that contains the `core-host` binary, the guest WASM artifact, and exposes port `8080`.

## 2. Kubernetes Manifests

- [ ] 2.1 Add `manifests/deploy.yaml` with a `Deployment` named `tachyon-host` that uses `tachyon-mesh:test`.
- [ ] 2.2 Configure the deployment with `imagePullPolicy: Never` or `IfNotPresent` so the local k3d image import is used.
- [ ] 2.3 Add a `Service` named `tachyon-service` that exposes the host on port `8080`.

## 3. Integration Workflow

- [ ] 3.1 Add `.github/workflows/integration.yml` triggered by `pull_request` and `workflow_dispatch`.
- [ ] 3.2 Build the image, install `k3d`, create the test cluster, and import `tachyon-mesh:test` into it.
- [ ] 3.3 Apply the Kubernetes manifests and wait for `deployment/tachyon-host` to become available.
- [ ] 3.4 Port-forward or otherwise expose `tachyon-service` locally and assert that `curl http://localhost:8080/api/guest-example` returns a successful response.
