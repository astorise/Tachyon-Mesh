# k3d-integration-test Specification

## Purpose
TBD - created by archiving change k3d-integration-test. Update Purpose after archive.
## Requirements
### Requirement: Container build packages the host and guest for Kubernetes execution
The repository SHALL provide a multi-stage container build that compiles `guest-example` for `wasm32-wasip1`, compiles `core-host` for `x86_64-unknown-linux-musl`, and produces a minimal runtime image containing the host binary and guest WASM artifact.

#### Scenario: Docker build emits a deployable runtime image
- **WHEN** a developer or CI job runs `docker build -t tachyon-mesh:test .`
- **THEN** the builder stage installs the Rust targets required for the guest and static host builds
- **AND** `guest-example` is compiled for `wasm32-wasip1`
- **AND** `core-host` is compiled in release mode for `x86_64-unknown-linux-musl`
- **AND** the final image includes the host binary and the compiled guest artifact needed at runtime

### Requirement: Kubernetes manifests deploy the local test image without an external registry
The repository SHALL provide Kubernetes manifests that deploy the `tachyon-mesh:test` image to a cluster and expose the host on port `8080` without requiring the image to be pulled from a remote registry.

#### Scenario: Cluster consumes the imported local image
- **WHEN** the manifests are applied to a k3d cluster after importing `tachyon-mesh:test`
- **THEN** a `Deployment` named `tachyon-host` schedules a pod from that image
- **AND** the pod uses `imagePullPolicy: Never` or `IfNotPresent`
- **AND** a `Service` named `tachyon-service` exposes port `8080` for the host

### Requirement: Integration workflow verifies the deployed host through k3d
The repository SHALL provide a GitHub Actions workflow that builds the image, creates an ephemeral k3d cluster, deploys the manifests, waits for the host to become available, and verifies the `/api/guest-example` endpoint with an HTTP request.

#### Scenario: CI validates the cluster deployment path
- **WHEN** the integration workflow runs on GitHub Actions
- **THEN** it creates a disposable k3d cluster
- **AND** it imports the locally built `tachyon-mesh:test` image into that cluster
- **AND** it applies the Kubernetes manifests and waits for `deployment/tachyon-host` to become available
- **AND** it issues an HTTP request to `/api/guest-example`
- **AND** the request succeeds only if the deployed host is serving traffic correctly

