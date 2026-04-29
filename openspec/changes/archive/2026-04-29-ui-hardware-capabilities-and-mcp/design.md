# Design: UI and MCP Capabilities Integration

## 1. Schema Definition (capabilities.json)
This file will act as the source of truth for both the rich client and the CLI. It will define the `HardwarePolicy` structure:
- `accelerators`: Ordered array of preferences (e.g., `["gpu", "npu", "cpu"]`).
- `min_ram_mb`: Integer requirement.
- `min_vram_mb`: Optional integer requirement.
- `qos_class`: Enum (`realtime`, `batch`, `background`).
- `admission_strategy`: Enum (`fail_fast`, `mesh_retry`).

## 2. Tachyon-UI (Schema-Driven UI)
During the "Deploy FaaS" view, the frontend will load `capabilities.json`.
- A dynamic form component will parse the JSON `type`, `minimum`, and `enum` fields to generate sliders (for RAM/VRAM) and dropdown menus (for QoS).
- If the backend emits a system pressure alert via WebSocket, the UI will gray out `realtime` options that demand too much RAM, guiding the user toward `batch` execution.

## 3. Model Context Protocol (MCP) Integration
The `tachyon-mcp` crate (allowing AI clients like Cursor or Claude to interact with Tachyon) will be updated:
- **Resource `hardware://local/status`**: Returns a real-time JSON object detailing free RAM/VRAM on the current Edge node.
- **Tool `validate_faas_capabilities`**: Allows an AI agent to simulate a FaaS manifest submission. The agent sends its JSON constraints, and the MCP server replies with a simulated approval or rejection from the Admission Control layer, preventing blind deployment errors.