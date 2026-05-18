# hygiene-plan Specification

## Purpose
TBD - created by archiving change v1-1-audit-hygiene. Update Purpose after archive.
## Requirements
### Requirement: Pre-release SemVer Suffix
All workspace manifests SHALL declare version `1.1.0-alpha` while the v1.1 feature set is incomplete.

#### Scenario: Cargo workspace reports pre-release version
- **GIVEN** the repository at the `v1.1.x` branch
- **WHEN** `cargo metadata --format-version 1` is invoked at the workspace root
- **THEN** every workspace member SHALL report version `1.1.0-alpha`

#### Scenario: Frontend manifests report pre-release version
- **GIVEN** `package.json` and `tachyon-ui/tauri.conf.json` exist
- **WHEN** the version field is read
- **THEN** both files SHALL contain `1.1.0-alpha`

### Requirement: Telemetry Mutex Poison Visibility
The telemetry registries in `core-host/src/telemetry/mod.rs` SHALL surface poisoned-lock recovery via an explicit warning log entry rather than silently returning the inner value.

#### Scenario: Poisoned lock emits diagnostic log
- **GIVEN** a `Mutex` in the telemetry registry has been poisoned by a panicking thread
- **WHEN** another thread attempts to acquire the lock
- **THEN** the recovery path SHALL log a warning that the registry was poisoned
- **AND** the recovery path SHALL include the registry identifier in the log payload

#### Scenario: Healthy lock acquisition stays silent
- **GIVEN** a telemetry registry lock has never been poisoned
- **WHEN** a thread acquires the lock normally
- **THEN** no poison-recovery log SHALL be emitted

