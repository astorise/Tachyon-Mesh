## ADDED Requirements

### Requirement: The desktop UI switches management planes without reloading
The `tachyon-ui` frontend SHALL bind sidebar navigation links to pre-rendered management-plane views and switch between them inside the existing `<main>` container without a full page reload.

#### Scenario: The operator selects a different management plane
- **WHEN** the operator clicks a sidebar link for Dashboard, Mesh Topology, FaaS Deployments, Identity, or AI Broker
- **THEN** the currently visible panel fades and slides out through GSAP
- **AND** the selected panel fades and slides in within the same page shell
- **AND** the selected sidebar link becomes the active link

### Requirement: The desktop UI exposes dedicated panels for topology, deployment, identity, and AI workflows
The `tachyon-ui` frontend SHALL expose dedicated panels for mesh topology, FaaS deployments, identity posture, and AI model brokerage using the shared Tauri commands and widgets already owned by the desktop client.

#### Scenario: The operator opens Mesh Topology
- **WHEN** the Mesh Topology panel becomes active
- **THEN** the frontend invokes `get_mesh_graph`
- **AND** it renders the returned route and batch-target snapshot in the topology view

#### Scenario: The operator opens FaaS Deployments
- **WHEN** the FaaS Deployments panel becomes active
- **THEN** the dashboard content is replaced by a panel labeled `Deployment Manager (Ready)`
- **AND** the asset upload controls remain available in that panel

#### Scenario: The operator opens Identity
- **WHEN** the Identity panel becomes active
- **THEN** the frontend renders the administrative user table and MFA recovery posture from the existing onboarding workflow

#### Scenario: The operator opens AI Broker
- **WHEN** the AI Broker panel becomes active
- **THEN** the frontend renders the chunked model upload controls and progress bar in that panel
