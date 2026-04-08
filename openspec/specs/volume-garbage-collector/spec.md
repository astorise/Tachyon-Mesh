# volume-garbage-collector Specification

## Purpose
TBD - created by archiving change volume-garbage-collector. Update Purpose after archive.
## Requirements
### Requirement: Volume mounts can define a TTL for garbage collection
The integrity manifest SHALL allow volume mount definitions to declare `ttl_seconds` so ephemeral data can be evicted automatically after it becomes stale.

#### Scenario: A volume mount declares a TTL
- **WHEN** a target mount specifies `ttl_seconds`
- **THEN** the host loads that TTL into the runtime configuration without breaking manifests that omit it

### Requirement: The host runs a periodic background sweeper for TTL-managed paths
The host SHALL run a periodic background task that collects unique TTL-managed host paths and delegates filesystem scanning to blocking worker threads.

#### Scenario: The sweeper tick runs
- **WHEN** the periodic garbage collector wakes up
- **THEN** it gathers the configured host paths with TTL settings
- **AND** scans them through blocking filesystem work so the async executor is not stalled

### Requirement: Stale files are removed based on modified time
The garbage collector SHALL delete files or directories whose age exceeds the configured TTL and SHALL tolerate races where files disappear or remain in active use.

#### Scenario: A file is older than the configured TTL
- **WHEN** the sweeper examines an entry whose last modified time is older than `ttl_seconds`
- **THEN** it deletes that entry and records the deletion in host logs

#### Scenario: A file cannot be removed because of a race
- **WHEN** the sweeper encounters a `NotFound` or `PermissionDenied` style error while deleting a stale entry
- **THEN** it handles the error gracefully without crashing the host

