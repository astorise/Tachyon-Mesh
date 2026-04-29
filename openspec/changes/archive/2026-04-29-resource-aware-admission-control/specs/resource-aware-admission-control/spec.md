## ADDED Requirements

### Requirement: Resource policy manifest fields
FaaS deployment manifests MUST support resource policy fields for minimum RAM, optional VRAM, and admission strategy.

#### Scenario: Manifest includes resource policy
- **GIVEN** a deployment manifest contains `resource_policy`
- **WHEN** the host and UI validate the manifest
- **THEN** `min_ram_gb` or equivalent RAM constraints are parsed
- **AND** `admission_strategy` is limited to supported values such as `fail_fast` and `mesh_retry`

### Requirement: Pre-instantiation admission check
The host MUST check available local resources before instantiating a workload.

#### Scenario: Local RAM is insufficient
- **GIVEN** a request targets a workload with a minimum RAM requirement
- **WHEN** local available RAM is below the requirement
- **THEN** the host does not instantiate the workload locally
- **AND** it follows the workload admission strategy

### Requirement: Mesh retry admission strategy
When configured for mesh retry, the host MUST attempt to offload a rejected request to a capable peer before failing the request.

#### Scenario: Capable neighbor exists
- **GIVEN** local resources are insufficient for a request
- **AND** the admission strategy is `mesh_retry`
- **WHEN** a neighboring node advertises enough available capacity
- **THEN** the host proxies the request to that neighbor through the secure mesh path
- **AND** it returns the neighbor response to the original caller

### Requirement: Saturation feedback
The host and UI MUST surface clear saturation feedback for rejected synchronous work and delayed asynchronous work.

#### Scenario: No node can accept work
- **GIVEN** no local or neighboring node satisfies a workload resource policy
- **WHEN** a synchronous request is evaluated
- **THEN** the host returns HTTP 503
- **AND** it includes `X-Tachyon-Reason: Insufficient-Cluster-Resources`
