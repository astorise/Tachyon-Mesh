# homelab-auth-state-persistence Specification

## Purpose
Define Home Lab deployment requirements for preserving Tachyon AuthN account state across pod replacement and image upgrades.

## Requirements
### Requirement: Home Lab persists AuthN account state outside the pod filesystem
The Home Lab Kubernetes deployment SHALL mount persistent storage at `/app/auth-state` for `core-host` so AuthN account, token, and enrollment state survive pod replacement and image upgrades.

#### Scenario: AuthN state volume is provisioned
- **WHEN** the Home Lab manifest is applied
- **THEN** Kubernetes creates or reuses a persistent volume claim for AuthN state
- **AND** the `core-host` container mounts that claim at `/app/auth-state`

#### Scenario: Account state survives deployment rollout
- **WHEN** the `tachyon-host` deployment rolls out a new image
- **THEN** the replacement pod mounts the same AuthN state volume
- **AND** existing account state remains available to AuthN after the rollout

#### Scenario: Missing persistent volume is treated as deployment invalid
- **WHEN** the Home Lab deployment is reviewed for production-like use
- **THEN** a `tachyon-host` pod without a persistent `/app/auth-state` mount is considered invalid
- **AND** the deployment must be corrected before relying on enrolled accounts
