## MODIFIED Requirements

### Requirement: Container build packages the host and guest for Kubernetes execution
The repository SHALL provide a multi-stage container build that compiles `guest-example` and `guest-call-legacy` for `wasm32-wasip1`, compiles `core-host` and `legacy-mock` for `x86_64-unknown-linux-musl`, and produces deployable runtime images for both the host and the legacy service.

#### Scenario: Docker builds emit deployable host and legacy images
- **WHEN** a developer or CI job runs `docker build -t tachyon-mesh:test .`
- **AND** a developer or CI job runs `docker build --target legacy-runtime -t legacy-mock:test .`
- **THEN** the builder stage installs the Rust targets required for guest and static host builds
- **AND** both guest modules are compiled for `wasm32-wasip1`
- **AND** both native binaries are compiled in release mode for `x86_64-unknown-linux-musl`
- **AND** the host runtime image includes the `core-host` binary and both guest WASM artifacts
- **AND** the legacy runtime image includes the `legacy-mock` binary

### Requirement: Kubernetes manifests deploy the local test image without an external registry
The repository SHALL provide Kubernetes manifests that deploy both `tachyon-host` and `legacy-deployment` from locally imported images and expose them as `tachyon-service` on port `8080` and `legacy-service` on port `8081`.

#### Scenario: Cluster consumes the imported local images
- **WHEN** the manifests are applied to a k3d cluster after importing `tachyon-mesh:test` and `legacy-mock:test`
- **THEN** a `Deployment` named `tachyon-host` schedules a pod from the host image
- **AND** a `Deployment` named `legacy-deployment` schedules a pod from the legacy image
- **AND** the workloads use `imagePullPolicy: Never` or `IfNotPresent`
- **AND** `tachyon-service` exposes port `8080`
- **AND** `legacy-service` exposes port `8081`

### Requirement: Integration workflow verifies the deployed host through k3d
The repository SHALL provide a GitHub Actions workflow that builds both images, creates an ephemeral k3d cluster, deploys both services, and validates direct and bridged traffic across the hybrid mesh topology.

#### Scenario: CI validates direct and bridged traffic paths
- **WHEN** the integration workflow runs on GitHub Actions
- **THEN** it creates a disposable k3d cluster
- **AND** it imports the locally built `tachyon-mesh:test` and `legacy-mock:test` images into that cluster
- **AND** it applies the Kubernetes manifests and waits for both deployments to become available
- **AND** it verifies `GET /ping` against the legacy service
- **AND** it verifies a direct FaaS request to `/api/guest-example`
- **AND** it verifies `POST /call-faas` on the legacy service returns the FaaS response
- **AND** it verifies `/api/guest-call-legacy` returns the legacy response through the host bridge
