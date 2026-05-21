## ADDED Requirements

### Requirement: CI validates the s3-persistence feature build
The CI workflow SHALL include a `cargo check -p core-host --features s3-persistence` step in the `rust-ci` job to verify the feature compiles without errors on every push.

#### Scenario: s3-persistence check runs in rust-ci
- **WHEN** the CI workflow runs on GitHub Actions
- **THEN** it runs `cargo check -p core-host --features s3-persistence`
- **AND** the step fails the build on any compilation error

### Requirement: feature-matrix-tests includes s3-persistence combination
The `feature-matrix-tests` job SHALL include `--features s3-persistence` as one of its matrix entries, uploading the resulting binary as a labeled artifact.

#### Scenario: s3-persistence matrix entry builds and uploads artifact
- **WHEN** the feature-matrix-tests job runs the s3-persistence entry
- **THEN** it runs `cargo test -p core-host --features s3-persistence` and `cargo build -p core-host --release --features s3-persistence`
- **AND** uploads the binary as `core-host-linux-x86_64-s3-persistence-<sha>`
