# Proposal: Real SemVer Dependency Resolution in the Bundle Pipeline

## Why

The initial smart deployment pipeline used `TACHYON_BUNDLE_FAKE_CONFLICTS`, an
environment variable, to simulate override conflicts during development. No
real SemVer comparison was performed: every apply succeeded with no conflict
detection regardless of what the cluster actually held.

The spec requires the host to detect when a bundled asset's version is shadowed
by a strictly higher, still-compatible version already present in the cluster
registry, and to halt the apply with HTTP 428 in that case.

## What Changes

- `IntegrityConfig` gains `asset_versions: BTreeMap<String, String>` (serde
  default empty). This map is persisted inside `integrity.lock` and updated by
  every successful bundle apply, recording the exact deployed version of each
  bundled asset so subsequent applies have a real cluster baseline to compare
  against.
- `detect_dependency_conflicts` is rewritten to use `semver::VersionReq` and
  `semver::Version` for a correct two-step check: (a) is the cluster version
  strictly greater than the bundled version, and (b) does the cluster version
  still satisfy the same SemVer constraint declared in the manifest? Only
  dependencies that carry a local `source` path are eligible for conflict
  detection.
- `extract_semver_version` strips range operators (`^`, `~`, `>=`, `=`, etc.)
  and returns the bare version string, used both for the conflict comparison
  and to populate `asset_versions` after a successful apply.
- `TACHYON_BUNDLE_FAKE_CONFLICTS` is removed; the env-var mock is no longer
  needed.
- Eight unit tests in `integrity_admin.rs` cover the full conflict-detection
  matrix (no cluster entry, cluster equals, cluster older, cluster
  incompatible, cluster newer, no source, multiple conflicts, operator strip).

## Non-goals

- A full OCI-compatible asset registry with tagging and promotion workflows.
- SemVer resolution for non-bundled dependencies (those without a local
  `source`); those are assumed to be cluster-resolved already.
