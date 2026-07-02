## Purpose

Define the Tachyon UI shell wiring requirements for route resolution, overview telemetry, and IAM operator staging workflows.

## Requirements

### Requirement: Web component registry includes topology route
The Tachyon web component registry SHALL map `topology` to a concrete custom element so shell navigation resolves without falling back to static or unknown route content.

#### Scenario: Topology route resolves through registry
- **WHEN** the shell asks `ComponentRegistry` to resolve `topology`
- **THEN** the registry returns `tachyon-topology-panel`
- **AND** the route label remains `Mesh Topology`

### Requirement: Web component registry excludes policy-only dead routes
The Tachyon web component registry SHALL expose only panels backed by runtime manifest fields, live admin APIs, or read-only telemetry. Legacy policy-only routes that previously wrote `ui_configurations` overlays SHALL NOT be reachable from shell navigation.

#### Scenario: Removed policy-only routes are not resolved
- **WHEN** the shell asks `ComponentRegistry` to resolve `rbac`, `fleet`, or `supply-chain`
- **THEN** the registry returns no component route
- **AND** operators use the runtime-backed Users & Groups IAM view or bundle apply workflow instead

#### Scenario: Routing dashboard legacy payload route is not imported
- **WHEN** the application shell initializes
- **THEN** it imports `tachyon-routing-panel` for the `routing` route
- **AND** it does not import or register the legacy `tachyon-routing-dashboard` form

### Requirement: Shell exposes optional chat micro-frontend
The Tachyon web component registry SHALL map `chat` to `tachyon-chat-panel`.
The chat panel SHALL attempt to load `/chat/tachyon-chat-assistant.js`
dynamically from the static FaaS endpoint and render the remote
`<tachyon-chat-assistant>` element when available. If the script cannot be
loaded, the shell SHALL keep rendering without throwing and SHALL show a
non-blocking unavailable state.

#### Scenario: Chat route resolves through registry
- **WHEN** the shell asks `ComponentRegistry` to resolve `chat`
- **THEN** the registry returns `tachyon-chat-panel`

#### Scenario: Missing static FaaS does not break the shell
- **WHEN** `<tachyon-chat-panel>` cannot import `/chat/tachyon-chat-assistant.js`
- **THEN** it renders a fallback unavailable state
- **AND** the rest of Tachyon-UI remains interactive

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

### Requirement: Bounded reconnection with user feedback
The UI network layer SHALL stop automatic reconnection after a fixed number of attempts and expose a manual retry control.

#### Scenario: Reconnect loop runs up to MAX_RETRIES
- **GIVEN** the cluster becomes unreachable
- **WHEN** the reconnect loop starts
- **THEN** it probes `get_engine_status` at most 5 times with exponential backoff
- **AND** each attempt updates `connectionStore` with `attempt` and `maxAttempts`
- **AND** `NetworkStatus` displays `"Reconnecting (N/5)"`

#### Scenario: Terminal disconnected state after exhausted retries
- **GIVEN** all 5 reconnect attempts have failed
- **WHEN** the reconnect loop finishes
- **THEN** `connectionStore.status` is set to `"disconnected"`
- **AND** `NetworkStatus` shows `"Cluster unreachable"` with a Retry button

#### Scenario: Manual retry starts a fresh cycle
- **GIVEN** the terminal disconnected state is active
- **WHEN** the operator clicks the Retry button
- **THEN** `connectionStore.manualRetry()` resets `attempt` to 0
- **AND** a new bounded cycle of up to 5 attempts begins
