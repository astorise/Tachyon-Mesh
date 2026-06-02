## MODIFIED Requirements

### Requirement: Node Enrollment MUST support declarative provisioning strategies
The `system-faas-enrollment` module SHALL rely on the declarative enrollment configuration to determine the provisioning strategy. When `enrollment.mode` is `zero-touch` or `both`, a freshly booted node SHALL attach a machine identity proof (a signed JWT such as a projected Kubernetes ServiceAccount token, read from its mounted token path) to `/admin/enrollment/start`. The approving node SHALL validate that token against the configured `oidc_issuer` and auto-assign the matched `auto_approve_tags` to the new node. When no machine identity is presented or its claims do not match, enrollment SHALL fall back to the operator-PIN approval path; PIN remains the strategy for edge/NAT nodes.

#### Scenario: Zero-Touch Provisioning of a new Edge node
- **GIVEN** the mesh is configured with `device_flow` enrollment and an `auto_approve_tags` list containing `env=production`
- **WHEN** an unconfigured `core-host` boots and completes the OAuth2 device flow via the UI
- **THEN** it automatically receives the cryptographic certificates
- **AND** it is immediately tagged as a `production` node, subscribing only to production-grade GitOps updates.

#### Scenario: Zero-Touch Provisioning of a cluster pod
- **GIVEN** `enrollment.mode = "both"` and `auto_approve_tags` requiring the cluster's node ServiceAccount
- **WHEN** a pod boots without credentials and attaches its projected ServiceAccount token to `/admin/enrollment/start`
- **THEN** the approving node validates the token against `oidc_issuer` and auto-signs the CSR with no human step
- **AND** the new node is tagged with the matched `auto_approve_tags`

#### Scenario: Falls back to PIN when no identity matches
- **GIVEN** `enrollment.mode = "both"`
- **WHEN** a node presents no machine identity (or one whose claims do not match `auto_approve_tags`)
- **THEN** enrollment proceeds through the existing operator-PIN approval path
