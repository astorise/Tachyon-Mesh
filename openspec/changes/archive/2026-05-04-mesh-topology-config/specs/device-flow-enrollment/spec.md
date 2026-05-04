# device-flow-enrollment Delta

## ADDED Requirements

### Requirement: Node Enrollment MUST support declarative provisioning strategies
The `system-faas-enrollment` module SHALL rely on the declarative `topology-configuration` to determine which Identity Provider to contact and what metadata tags to auto-assign to new nodes.

#### Scenario: Zero-Touch Provisioning of a new Edge node
- **GIVEN** the mesh is configured with `device_flow` enrollment and an `auto_approve_tags` list containing `env=production`
- **WHEN** an unconfigured `core-host` boots and completes the OAuth2 device flow via the UI
- **THEN** it automatically receives the cryptographic certificates
- **AND** it is immediately tagged as a `production` node, subscribing only to production-grade GitOps updates.
