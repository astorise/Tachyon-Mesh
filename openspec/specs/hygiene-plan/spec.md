# hygiene-plan Specification

## Purpose
TBD - created by archiving change v1-1-audit-hygiene. Update Purpose after archive.
## Requirements
<!--
TACH-36 (2026-10-29 audit): removed the "Pre-release SemVer Suffix"
requirement that mandated `1.1.0-alpha` across every workspace manifest
"while the v1.1 feature set is incomplete". The workspace moved past that
pre-release phase and settled on `1.0.0` as its real v1.0.0 baseline
(core-host, tachyon-ui's Cargo.toml/tauri.conf.json/package.json, and the
SDKs all report `1.0.0` -- see TACH-28), so this requirement had become
permanently false rather than describing a real, current rule. There is no
longer a `v1.1.x` branch for it to apply to. If a future pre-1.0 branch
needs an equivalent rule, open a new openspec change scoped to that branch
rather than reviving this one.
-->
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

