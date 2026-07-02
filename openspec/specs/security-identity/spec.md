# security-identity Specification

## Purpose
Expose security identity and RBAC configuration dashboards in the Tachyon web component shell.

## Requirements
### Requirement: Web component shell exposes Identity and Quotas configuration
The Tachyon web component shell SHALL provide a `<tachyon-identity-panel>` dashboard that extends `TachyonConfigDashboard` and lets an operator manage trusted manifest signers through the runtime manifest.

#### Scenario: Operator submits trusted signers
- **WHEN** the operator enters trusted Ed25519 public keys in the Identity panel
- **THEN** the panel validates each key as 64-character hex
- **AND** writes `trusted_signers` through `apply_manifest_config`
- **AND** the panel renders success or error feedback using `showFeedback`

#### Scenario: Identity panel is reachable from shell navigation
- **WHEN** the authenticated shell renders its configuration navigation
- **THEN** it includes an Identity & Quotas route
- **AND** selecting that route mounts `<tachyon-identity-panel>`

### Requirement: Web component shell exposes RBAC through Users and Groups
The Tachyon web component shell SHALL expose RBAC administration through the runtime-backed Users & Groups panel and SHALL NOT route a separate legacy `<tachyon-rbac-panel>` policy form.

#### Scenario: Operator updates RBAC group
- **WHEN** the operator edits a group in Users & Groups
- **THEN** the panel invokes the IAM group admin command
- **AND** it does not submit a legacy RBAC domain payload

#### Scenario: RBAC policy form is not reachable from shell navigation
- **WHEN** the authenticated shell renders its configuration navigation
- **THEN** it does not include an RBAC route
- **AND** no `<tachyon-rbac-panel>` is mounted
