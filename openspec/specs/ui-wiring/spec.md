## Purpose

Define the Tachyon UI shell wiring requirements for route resolution, overview telemetry, and IAM operator staging workflows.

## Requirements

### Requirement: Web component registry includes topology and registry routes
The Tachyon web component registry SHALL map `topology` and `registry` routes to concrete custom elements so shell navigation resolves without falling back to static or unknown route content.

#### Scenario: Topology route resolves through registry
- **WHEN** the shell asks `ComponentRegistry` to resolve `topology`
- **THEN** the registry returns `tachyon-topology-panel`
- **AND** the route label remains `Mesh Topology`

#### Scenario: Registry route resolves through registry
- **WHEN** the shell asks `ComponentRegistry` to resolve `registry`
- **THEN** the registry returns `tachyon-supply-chain-panel`
- **AND** the route label remains `Asset Registry`

### Requirement: Overview dashboard uses live mesh graph telemetry
The `<tachyon-overview-panel>` dashboard SHALL fetch telemetry from the Tauri `get_mesh_graph` command and update its counters asynchronously before animating them.

#### Scenario: Overview updates counters from mesh graph
- **WHEN** `<tachyon-overview-panel>` connects
- **THEN** it invokes `get_mesh_graph`
- **AND** it maps the returned routes and batch targets into displayed counter values
- **AND** GSAP animates the counters to the fetched values

#### Scenario: Overview handles telemetry fetch failure
- **WHEN** `get_mesh_graph` fails
- **THEN** the overview panel renders an error state
- **AND** it dispatches a global notification event

### Requirement: IAM component can stage a new operator
The `<tachyon-iam>` component SHALL render a Stage New Operator form and invoke `stage_signup` with node URL, enrollment token, first name, last name, username, password, and null certificate values.

#### Scenario: Operator staging form submits stage signup payload
- **WHEN** the operator submits the Stage New Operator form with all required fields
- **THEN** `<tachyon-iam>` invokes `stage_signup`
- **AND** the payload contains `url`, `token`, `firstName`, `lastName`, `username`, `password`, and `cert`
- **AND** a success notification is dispatched when staging succeeds

#### Scenario: Operator staging failure emits notification
- **WHEN** `stage_signup` rejects the submitted staging payload
- **THEN** `<tachyon-iam>` dispatches an error notification
- **AND** the component remains interactive
