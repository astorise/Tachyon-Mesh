## MODIFIED Requirements

### Requirement: Enrollment ceremony runs inside the FaaS

The operator-side enrollment ceremony (PIN generation, CSR signing, approval state machine) SHALL execute inside the `system-faas-node-registry` WASM component, built against the `control-plane-faas` WIT world. The same applies to the **machine-identity auto-approval branch**: JWT/OIDC validation, `auto_approve_tags` matching, and the resulting CSR signing SHALL also run inside the FaaS. `core-host` MUST NOT retain a parallel implementation of either branch; it SHALL only forward the relevant admin HTTP routes (`/admin/enrollment/*`, `/admin/nodes/*`) to the FaaS `handle-request` export.

#### Scenario: Approval routes are served by the FaaS

- **GIVEN** the host has loaded `system-faas-node-registry`
- **WHEN** an operator POSTs an approval to `/admin/enrollment/approve/{session_id}`
- **THEN** the host forwards the request to the FaaS `handle-request` export
- **AND** the FaaS performs the PIN check, signs the CSR, and returns the signed certificate in the response
- **AND** no `core-host` Rust module retains the approval state

#### Scenario: Machine-identity auto-approval is served by the FaaS

- **GIVEN** the host has loaded `system-faas-node-registry` and `enrollment.mode` allows zero-touch
- **WHEN** a node POSTs `/admin/enrollment/start` carrying a machine-identity JWT
- **THEN** the host forwards the request to the FaaS, which validates the token, matches `auto_approve_tags`, signs the CSR, and returns the certificate
- **AND** no `core-host` Rust module performs the token validation or signing

### Requirement: Persistent enrolled-node registry

The `system-faas-node-registry` component SHALL persist a record for every node whose enrollment has been approved, keyed by the node's stable `node_id`. Persistence MUST go through the `kv-partition::table` WIT resource on a table named `"node-registry"`, which the host backs with a dedicated ReDB table inside `CoreStore`. Each record MUST survive host process restarts. Each record MUST also capture approval provenance: an `approved_by` value of the form `pin:<operator>` or `oidc:<subject>`, and the list of `auto_approve_tags` that matched (empty for PIN approvals).

#### Scenario: Approved enrollment is persisted through kv-partition

- **WHEN** the FaaS completes an enrollment approval
- **THEN** it invokes `kv-partition::table::set` on the `"node-registry"` table with the `node_id` as key and a serialized `EnrolledNode` record as value
- **AND** subsequent calls to `list-enrolled-nodes` return the new entry with `status = "awaiting-capabilities"`
- **AND** the record contains the node's public key, the issuing operator's identity, and the approval timestamp

#### Scenario: Auto-approval records its provenance

- **WHEN** the FaaS auto-approves a node from a validated machine identity
- **THEN** the persisted `EnrolledNode` record sets `approved_by = "oidc:<subject>"` and lists the matched `auto_approve_tags`
- **AND** the approval emits a security/audit event identifying the subject and matched tags

#### Scenario: Registry survives host restart

- **GIVEN** at least one approved node is present in the registry
- **WHEN** the host process restarts and reloads `system-faas-node-registry`
- **THEN** the registry MUST return the same node entries on first read after restart
- **AND** the `status` field MUST be set to `"unknown"` for every entry until a heartbeat refreshes it
