# wit-oci-registry Specification

## Purpose
Distributes Tachyon Mesh WIT interface contracts as OCI artifacts to GitHub Container Registry, enabling FaaS guest developers to consume them via `cargo-component` without local `.wit` files.

## Requirements

### Requirement: WIT contracts MUST be published to GHCR on every version tag
`.github/workflows/publish-sdks.yml` SHALL contain a `publish-wit-oci` job that triggers on `v*` tags and `release: published` events. The job SHALL use `wkg publish` to push `./wit` as an OCI artifact to `ghcr.io/$OWNER/tachyon-mesh-wit:$VERSION`.

#### Scenario: Tag push triggers WIT publication
- **GIVEN** a `v1.2.3` tag is pushed
- **WHEN** the `publish-wit-oci` job runs
- **THEN** `wkg publish` is called with `--version 1.2.3` (no `v` prefix) and the artifact is available at `ghcr.io/astorise/tachyon-mesh-wit:1.2.3`

### Requirement: GHCR authentication MUST use GITHUB_TOKEN with packages:write permission
The `publish-wit-oci` job SHALL declare `permissions: packages: write` and authenticate via `docker/login-action` with `password: ${{ secrets.GITHUB_TOKEN }}`.

#### Scenario: Workflow authenticates to GHCR
- **WHEN** the `publish-wit-oci` job starts
- **THEN** it requests `packages: write` permission
- **AND** it authenticates to GHCR with `GITHUB_TOKEN`

### Requirement: Guest developers MUST be documented on OCI consumption
`README.md` SHALL include a section explaining `[package.metadata.component.dependencies]` syntax for `cargo-component`. `faas-sdk/README.md` SHALL document both the SDK crate path and the WIT/OCI path.

#### Scenario: Developer follows WIT OCI documentation
- **WHEN** a guest developer reads the SDK documentation
- **THEN** they can find both the local SDK crate path and the GHCR WIT OCI dependency syntax
