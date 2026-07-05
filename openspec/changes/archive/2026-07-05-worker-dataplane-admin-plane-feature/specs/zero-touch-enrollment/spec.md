## MODIFIED Requirements

### Requirement: Cluster bootstrap discovery for automated deployments
For automated multi-node deployments the enrollment bootstrap endpoint SHALL be resolvable without a designated master: `enrollment_endpoint` points at a stable cluster address (e.g. a Kubernetes headless Service backing a StatefulSet) so any ready peer can serve `/admin/enrollment/start`. This SHALL hold regardless of whether a given peer was built with the `admin-plane` Cargo feature enabled — a worker-profile peer (built with `admin-plane` disabled, see the `worker-dataplane-profile` capability) still mounts `/admin/enrollment/start` and `/admin/enrollment/poll/{session_id}` unconditionally and can serve an enrolling node's bootstrap request. A seed node SHALL be able to obtain signing authority from a mounted cluster-CA secret (or a pre-sealed manifest) so the first node enrolls without a peer to ask.

#### Scenario: New pod enrolls against any ready peer
- **GIVEN** a StatefulSet of nodes behind a headless Service and `enrollment_endpoint` set to that Service
- **WHEN** a new pod boots without credentials
- **THEN** it opens the outbound enrollment tunnel to any ready peer resolved through the Service and completes enrollment without contacting a designated master

#### Scenario: Seed node self-approves
- **GIVEN** the seed node mounts the cluster-CA private key from a secret (or boots pre-sealed)
- **WHEN** it starts with no peer available to approve it
- **THEN** it establishes its own credentials from the mounted authority and becomes a valid signer for subsequent nodes

#### Scenario: New pod enrolls against a worker-profile peer
- **GIVEN** a mixed cluster where some peers are built with the `admin-plane` Cargo feature disabled
- **WHEN** a new pod's outbound enrollment call happens to resolve to one of those worker-profile peers
- **THEN** it still completes `/admin/enrollment/start` and `/admin/enrollment/poll/{session_id}` against that peer exactly as it would against an admin-plane peer
