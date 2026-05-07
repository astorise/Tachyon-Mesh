## ADDED Requirements

### Requirement: Web component shell shows a global telemetry overview after login
The Tachyon web component shell SHALL provide a `<tachyon-overview-panel>` dashboard that extends `TachyonConfigDashboard` and is automatically mounted into `#router-view` after a successful `iam:authenticated` event.

#### Scenario: Overview is mounted after authentication
- **WHEN** the IAM layer emits `iam:authenticated`
- **THEN** `TachyonAppShell` displays the shell
- **AND** it mounts `<tachyon-overview-panel>` into the router view without requiring a sidebar click
- **AND** the overview navigation item is marked active

#### Scenario: Overview route is reachable from navigation
- **WHEN** the authenticated shell renders navigation links
- **THEN** it includes an Overview route
- **AND** selecting that route mounts `<tachyon-overview-panel>`

### Requirement: Global overview animates mesh telemetry counters
The `<tachyon-overview-panel>` dashboard SHALL render metric cards for active edge nodes, global Wasm instances, and AI/GPU utilization, and SHALL animate numeric counters from zero to their displayed values using GSAP.

#### Scenario: Counters animate on panel mount
- **WHEN** `<tachyon-overview-panel>` is connected
- **THEN** it renders a responsive grid of telemetry metric cards
- **AND** each numeric counter starts at zero
- **AND** GSAP animates each counter to its configured value

#### Scenario: Overview uses established visual styling
- **WHEN** the overview panel renders
- **THEN** it uses dark slate backgrounds, cyan highlights, and monospace numeric data
- **AND** it inherits shared dashboard styling through `TachyonConfigDashboard`
