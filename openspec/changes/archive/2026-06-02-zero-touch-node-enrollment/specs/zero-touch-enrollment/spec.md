## ADDED Requirements

### Requirement: Machine-identity enrollment via OIDC-validated token
A node MAY present a machine identity proof — a signed JWT (e.g. a projected Kubernetes ServiceAccount token) — on `/admin/enrollment/start`. The `system-faas-node-registry` FaaS SHALL validate the token against the configured `oidc_issuer` (JWKS signature, `aud` against `oidc_audience`, and expiry with small leeway) before trusting any of its claims. Validation MUST run inside the FaaS; `core-host` only forwards the route.

#### Scenario: Valid machine identity is accepted for evaluation
- **WHEN** a node POSTs `/admin/enrollment/start` with a JWT whose signature verifies against the issuer JWKS, whose `aud` matches `oidc_audience`, and which is unexpired
- **THEN** the FaaS extracts the token claims and proceeds to auto-approve evaluation

#### Scenario: Unverifiable token is rejected
- **WHEN** the presented token fails signature, audience, or expiry verification
- **THEN** the FaaS does NOT treat it as an identity and does not auto-approve on its basis
- **AND** the request falls back to the PIN path

#### Scenario: Issuer unreachable fails closed to PIN
- **WHEN** the FaaS cannot fetch the issuer JWKS
- **THEN** it does NOT auto-approve and the enrollment falls back to the PIN path

### Requirement: Auto-approval is gated by identity claims, never by network position
The FaaS SHALL auto-sign a node CSR only when the validated token claims match every matcher in the configured `auto_approve_tags` list. Network position (source address, in-cluster reachability) alone SHALL NOT authorize auto-approval.

#### Scenario: Matching claims auto-sign the CSR
- **GIVEN** `auto_approve_tags = ["namespace=tachyon", "serviceaccount=tachyon-node"]`
- **WHEN** a node presents a valid token whose claims satisfy both matchers
- **THEN** the FaaS signs the node CSR with the cluster CA and returns the certificate down the outbound tunnel with no human step

#### Scenario: Non-matching claims fall back to PIN
- **GIVEN** `auto_approve_tags` requires `namespace=tachyon`
- **WHEN** a node presents a valid token whose `namespace` claim is `default`
- **THEN** the FaaS does NOT auto-approve
- **AND** the enrollment falls back to operator PIN approval

### Requirement: Enrollment configuration block
The `IntegrityConfig` SHALL support an optional `enrollment` block with `mode` (`pin` | `zero-touch` | `both`), `oidc_issuer`, `oidc_audience`, and `auto_approve_tags` (a list of `key=value` claim matchers). When the block is absent, `mode` SHALL default to `pin` and behavior SHALL be identical to PIN-only enrollment. Validation SHALL reject `zero-touch` or `both` mode when `oidc_issuer` is empty, and SHALL reject `auto_approve_tags` entries that are not `key=value`.

#### Scenario: Zero-touch without issuer is rejected
- **WHEN** a manifest sets `enrollment.mode = "zero-touch"` with an empty `oidc_issuer`
- **THEN** configuration validation fails with an error naming the missing `oidc_issuer`

#### Scenario: Absent block preserves PIN behavior
- **WHEN** a manifest contains no `enrollment` block
- **THEN** the node enrolls exactly as in the PIN-only flow (no machine-identity path is taken)

### Requirement: Cluster bootstrap discovery for automated deployments
For automated multi-node deployments the enrollment bootstrap endpoint SHALL be resolvable without a designated master: `enrollment_endpoint` points at a stable cluster address (e.g. a Kubernetes headless Service backing a StatefulSet) so any ready peer can serve `/admin/enrollment/start`. A seed node SHALL be able to obtain signing authority from a mounted cluster-CA secret (or a pre-sealed manifest) so the first node enrolls without a peer to ask.

#### Scenario: New pod enrolls against any ready peer
- **GIVEN** a StatefulSet of nodes behind a headless Service and `enrollment_endpoint` set to that Service
- **WHEN** a new pod boots without credentials
- **THEN** it opens the outbound enrollment tunnel to any ready peer resolved through the Service and completes enrollment without contacting a designated master

#### Scenario: Seed node self-approves
- **GIVEN** the seed node mounts the cluster-CA private key from a secret (or boots pre-sealed)
- **WHEN** it starts with no peer available to approve it
- **THEN** it establishes its own credentials from the mounted authority and becomes a valid signer for subsequent nodes
