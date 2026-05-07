# security-identity Specification

## Purpose
Expose security identity and RBAC configuration dashboards in the Tachyon web component shell.

## Requirements
### Requirement: Web component shell exposes Identity and Quotas configuration
The Tachyon web component shell SHALL provide a `<tachyon-identity-panel>` dashboard that extends `TachyonConfigDashboard` and lets an operator submit JWT issuer and distributed CRDT quota configuration through the shared configuration command.

#### Scenario: Operator submits identity configuration
- **WHEN** the operator enters a JWT issuer URL and CRDT quota value in the Identity & Quotas panel
- **THEN** the panel invokes `apply_configuration` with the security identity domain
- **AND** the payload includes the issuer and numeric quota values
- **AND** the panel renders success or error feedback using `showFeedback`

#### Scenario: Identity panel is reachable from shell navigation
- **WHEN** the authenticated shell renders its configuration navigation
- **THEN** it includes an Identity & Quotas route
- **AND** selecting that route mounts `<tachyon-identity-panel>`

### Requirement: Web component shell exposes RBAC policy configuration
The Tachyon web component shell SHALL provide a `<tachyon-rbac-panel>` dashboard that extends `TachyonConfigDashboard` and lets an operator submit a selected role and structured policy payload through the shared configuration command.

#### Scenario: Operator submits RBAC policy
- **WHEN** the operator selects a role and enters a valid JSON policy document
- **THEN** the panel invokes `apply_configuration` with the RBAC domain
- **AND** the payload includes the selected role and parsed policy document
- **AND** the panel renders success or error feedback using `showFeedback`

#### Scenario: Invalid RBAC policy is rejected client side
- **WHEN** the operator submits malformed JSON in the RBAC policy field
- **THEN** the panel shows an error with `showFeedback`
- **AND** it does not invoke `apply_configuration`

#### Scenario: RBAC panel is reachable from shell navigation
- **WHEN** the authenticated shell renders its configuration navigation
- **THEN** it includes an RBAC route
- **AND** selecting that route mounts `<tachyon-rbac-panel>`
