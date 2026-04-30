## ADDED Requirements

### Requirement: README documents current delivery state
The repository README SHALL distinguish completed Tachyon Mesh phases from future roadmap work and SHALL document a Bash/Zsh-compatible startup path for using a sealed `integrity.lock`.

#### Scenario: Operator follows the README quick start
- **GIVEN** a signed `integrity.lock` exists at the repository root
- **WHEN** an operator follows the Quick Start shell commands
- **THEN** the manifest path is exported through `TACHYON_INTEGRITY_MANIFEST`
- **AND** the host can be started with `cargo run -p core-host --release`

### Requirement: Benchmarks are reproducible from committed harness files
The repository SHALL include a `bench/` harness that provisions a clean local Kubernetes environment, deploys neutral echo workloads for Tachyon Mesh, Istio Ambient, and Linkerd, runs Fortio latency tests, captures Kubernetes resource snapshots, and renders a Markdown report from raw results.

#### Scenario: Engineer generates benchmark artifacts
- **GIVEN** the required benchmark tools are installed
- **WHEN** the engineer runs the documented `bench/` workflow
- **THEN** raw Fortio JSON files are written under `bench/results/raw/`
- **AND** `bench/results/report.md` is generated from those raw files

### Requirement: Public performance claims are traceable to raw data
Published latency or memory comparisons SHALL be backed by committed raw benchmark artifacts and a recorded environment profile.

#### Scenario: No raw benchmark files exist
- **GIVEN** no Fortio JSON files exist under `bench/results/raw/`
- **WHEN** the report generator runs
- **THEN** it writes a report stating that no benchmark numbers are available
- **AND** it does not fabricate latency or memory values
