## MODIFIED Requirements

### Requirement: CI runs a dedicated feature-matrix test job across multiple feature flag combinations
The CI workflow SHALL run a `feature-matrix-tests` job that tests `core-host` across at least six distinct feature flag combinations including default, `--no-default-features`, `--all-features`, `--features http3`, a security bundle, and a worker data-plane profile (`--no-default-features` plus the transport features a mesh member needs with `admin-plane` omitted), uploading a release binary artifact for each combination.

#### Scenario: All feature combinations build and test successfully
- **WHEN** the feature-matrix-tests job runs
- **THEN** each matrix entry runs `cargo test -p core-host <features>` and `cargo build -p core-host --release <features>`
- **AND** each produces an uploaded artifact named `core-host-linux-x86_64-<label>-<sha>`

#### Scenario: All-features combination installs FIPS build dependencies
- **WHEN** the matrix entry with `--all-features` runs
- **THEN** it installs `cmake nasm protobuf-compiler` via apt before building

#### Scenario: Worker profile combination has no admin surface
- **WHEN** the worker-profile matrix entry runs
- **THEN** it builds and tests `core-host` with `admin-plane` disabled
- **AND** the router-level test suite confirms `/admin/*` routes other than enrollment bootstrap are unreachable
