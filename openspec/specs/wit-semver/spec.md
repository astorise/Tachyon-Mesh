# wit-semver Specification

## Purpose
Define semantic-version compatibility rules for Tachyon WIT interfaces used between the host and guest components.

## Requirements
### Requirement: Versioned WIT packages
Every public WIT package SHALL declare an explicit semantic version.

#### Scenario: WIT package is reviewed
- **WHEN** a WIT file is added or modified
- **THEN** its package declaration includes a semantic version compatible with the published contract

### Requirement: Breaking-change enforcement
The CI pipeline SHALL reject backward-incompatible WIT changes unless the major version is bumped.

#### Scenario: Field is removed without a major bump
- **WHEN** compatibility checks compare the current branch against the main branch
- **THEN** the workflow fails and reports the incompatible WIT change
