## ADDED Requirements

### Requirement: Web component shell exposes Storage configuration
The Tachyon web component shell SHALL provide a `<tachyon-storage-panel>` dashboard that extends `TachyonConfigDashboard` and lets an operator submit WASI volume mount path and S3 proxy endpoint settings through the shared configuration command.

#### Scenario: Operator submits storage configuration
- **WHEN** the operator enters a WASI volume mount path and S3 proxy endpoint
- **THEN** the panel invokes `apply_configuration` with the storage domain
- **AND** the payload includes the mount path and endpoint values
- **AND** the panel renders success or error feedback using `showFeedback`

#### Scenario: Storage panel is reachable from shell navigation
- **WHEN** the authenticated shell renders its configuration navigation
- **THEN** it includes a Storage route
- **AND** selecting that route mounts `<tachyon-storage-panel>`

### Requirement: Web component shell exposes Fleet configuration
The Tachyon web component shell SHALL provide a `<tachyon-fleet-panel>` dashboard that extends `TachyonConfigDashboard` and lets an operator submit node selector tags and a node profile through the shared configuration command.

#### Scenario: Operator submits fleet selector configuration
- **WHEN** the operator enters node selector tags and selects a node profile
- **THEN** the panel invokes `apply_configuration` with the fleet domain
- **AND** the payload includes selector tags and profile values
- **AND** the panel renders success or error feedback using `showFeedback`

#### Scenario: Fleet panel is reachable from shell navigation
- **WHEN** the authenticated shell renders its configuration navigation
- **THEN** it includes a Fleet route
- **AND** selecting that route mounts `<tachyon-fleet-panel>`

### Requirement: Web component shell exposes Supply Chain configuration
The Tachyon web component shell SHALL provide a `<tachyon-supply-chain-panel>` dashboard that extends `TachyonConfigDashboard` and lets an operator submit asset signature key and air-gapped mode settings through the shared configuration command.

#### Scenario: Operator submits supply chain configuration
- **WHEN** the operator enters an asset signature key and toggles air-gapped mode
- **THEN** the panel invokes `apply_configuration` with the supply chain domain
- **AND** the payload includes the signature key and air-gapped mode flag
- **AND** the panel renders success or error feedback using `showFeedback`

#### Scenario: Supply chain panel is reachable from shell navigation
- **WHEN** the authenticated shell renders its configuration navigation
- **THEN** it includes a Supply Chain route
- **AND** selecting that route mounts `<tachyon-supply-chain-panel>`
