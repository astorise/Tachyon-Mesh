## ADDED Requirements

### Requirement: Hardware capability schema
The UI capability schema MUST define hardware policy fields for accelerator preference, RAM, VRAM, QoS, and admission strategy.

#### Scenario: Deployment form loads capabilities schema
- **GIVEN** Tachyon UI opens the deploy workflow
- **WHEN** it loads `tachyon-ui/gen/schemas/capabilities.json`
- **THEN** the schema exposes `HardwarePolicy`
- **AND** the fields include accelerators, minimum RAM, optional minimum VRAM, QoS class, and admission strategy

### Requirement: Schema-driven hardware form
The deploy workflow MUST render hardware policy controls from the capability schema.

#### Scenario: User configures hardware policy
- **GIVEN** the hardware capability schema is available
- **WHEN** the deploy form renders
- **THEN** it presents numeric controls for RAM and VRAM
- **AND** it presents constrained selectors for accelerators, QoS class, and admission strategy
- **AND** it serializes the selected policy into the deployment manifest

### Requirement: MCP hardware telemetry resource
The MCP server MUST expose a hardware telemetry resource for local node capacity.

#### Scenario: MCP client reads hardware status
- **GIVEN** an MCP client is connected to `tachyon-mcp`
- **WHEN** it reads `hardware://local/status`
- **THEN** the server returns current local RAM and accelerator availability in JSON
- **AND** the response can be used to size a FaaS deployment manifest

### Requirement: MCP capability validation tool
The MCP server MUST provide a tool that validates draft FaaS hardware policies before deployment.

#### Scenario: MCP client validates a draft manifest
- **GIVEN** an MCP client submits a draft manifest with hardware constraints
- **WHEN** it invokes `validate_faas_capabilities`
- **THEN** the server simulates the admission decision
- **AND** it returns approval or a structured rejection reason
