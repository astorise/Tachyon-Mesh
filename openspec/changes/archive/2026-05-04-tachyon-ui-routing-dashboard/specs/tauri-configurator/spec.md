# tauri-configurator Specification

## ADDED Requirements

### Requirement: UI Configuration Views MUST generate strict Schema payloads
The frontend views SHALL NOT use custom or intermediate data formats for submitting configurations. Forms and controllers MUST serialize DOM state directly into the standard GitOps JSON payloads defined by the `system-faas-config-api` WIT schemas (e.g., `routing.tachyon.io/v1alpha1`).

#### Scenario: Submitting a new route
- **WHEN** the user interacts with the "Routing & Gateways" dashboard and clicks "Deploy Configuration"
- **THEN** the Vanilla JS controller builds a JSON object matching the `TrafficConfiguration` schema
- **AND** sends this exact payload to the Tauri backend for pushing to the Edge node, ensuring total compatibility with the Rust data-plane.
