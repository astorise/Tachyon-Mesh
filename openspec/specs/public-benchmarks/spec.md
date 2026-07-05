# public-benchmarks Specification

## Purpose
TBD - created by archiving change readme-sync-and-public-benchmarks. Update Purpose after archive.
## Requirements
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

### Requirement: Bench harness includes a FaaS-chain regression scenario
The `bench/` harness SHALL include a 3-hop FaaS chain scenario (`guest-chain-a` -> `guest-chain-b` -> `guest-chain-c`, wired via `examples/guest-examples/manifest.json` and sealed into a bench-only `integrity.lock` by `scripts/seal-bench-manifest.js`) that measures the in-process mesh dispatch hop and compares it against the same chain with local dispatch forcibly disabled via `TACHYON_BENCH_FORCE_MESH_TRANSPORT`.

#### Scenario: Engineer runs the FaaS-chain scenario
- **GIVEN** the `guest-chain-a/b/c` routes are sealed into a running core-host
- **WHEN** `bench/run-faas-chain.sh` is run against `/api/guest-chain-a` with and without `TACHYON_BENCH_FORCE_MESH_TRANSPORT=1` set on the core-host process
- **THEN** two Fortio JSON files are written under `bench/results/raw/` (`faas-chain-in-process.json` and `faas-chain-forced-transport.json`)
- **AND** the generated report distinguishes the p50/p99 latency of the in-process hop from the forced-transport hop

#### Scenario: Report verifies the chain stayed in-process
- **GIVEN** core-host was started with `RUST_LOG` enabling `core_host::host_core::mesh_dispatch_metrics=debug`
- **WHEN** the in-process phase's captured `kubectl logs` dispatch-decision lines are parsed
- **THEN** the report computes the share of decisions with `mode="in_process"`
- **AND** it fails the check when that share is below `faas_chain_in_process_share_min_pct` in `bench/results/thresholds.json`

### Requirement: Bench harness includes a cold-start regression scenario
The `bench/` harness SHALL include a cold-start scenario that measures the first-invocation latency of `/api/guest-example` immediately after a cache-clearing restart, with and without the `ComponentInstancePre`/`ModuleInstancePre` cache (`TACHYON_BENCH_DISABLE_INSTANCE_PRE_CACHE`).

#### Scenario: Engineer runs the cold-start scenario
- **WHEN** `bench/run-cold-start.sh` samples `/api/guest-example` right after a pod restart, with and without `TACHYON_BENCH_DISABLE_INSTANCE_PRE_CACHE=1` set on the core-host process
- **THEN** latency samples are appended as JSON lines under `bench/results/raw/cold-start-<variant>.jsonl`
- **AND** the generated report computes p50/p99 latency per variant

### Requirement: A periodic CI job runs the regression scenarios and gates on thresholds
The repository SHALL define `.github/workflows/bench-regression.yml`, triggered only by `workflow_dispatch` and a weekly `schedule` (never on push or pull request), that builds a bench-only core-host image, runs the FaaS-chain and cold-start scenarios on a real k3d cluster, and fails the job when a numeric threshold recorded in `bench/results/thresholds.json` is breached.

#### Scenario: Weekly run publishes an artifact and supports threshold calibration
- **WHEN** the scheduled or manually dispatched workflow completes
- **THEN** `bench/results/**` is uploaded as a workflow artifact regardless of pass/fail
- **AND** a `null` threshold in `bench/results/thresholds.json` is reported as provisional and does not fail the job
- **AND** a numeric threshold that is exceeded fails the job

