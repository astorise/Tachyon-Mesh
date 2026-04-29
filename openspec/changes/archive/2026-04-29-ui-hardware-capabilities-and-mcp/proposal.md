# Proposal: UI Hardware Capabilities & MCP Integration

## Context
Tachyon Mesh's backend architecture already supports Hardware-Aware Routing and Quality of Service (QoS) management via explicit Wasm contracts (WITs). However, the `tachyon-ui/gen/schemas/capabilities.json` file is currently an empty stub. Consequently, the Tachyon-UI frontend cannot generate the forms needed for administrators to define hardware constraints (RAM, VRAM, GPU) when deploying a FaaS module. Furthermore, the `tachyon-mcp` server does not expose node hardware telemetry, preventing external AI agents from properly sizing manifests before deployment.

## Objective
1. Hydrate the `capabilities.json` schema with strict definitions of the hardware constraints supported by the system.
2. Dynamically generate resource configuration forms in Tachyon-UI based on this schema.
3. Extend the Tachyon MCP server to expose local hardware capabilities to AI agents.

## Scope
- Populate `tachyon-ui/gen/schemas/capabilities.json`.
- Create a `HardwareCapabilitiesForm` React/Vue component within the UI.
- Add hardware-related `resources` and `tools` in the `tachyon-mcp` crate.