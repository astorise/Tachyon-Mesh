# Tasks: UI & MCP Capabilities Implementation

**Agent Instruction:** Implement the frontend and MCP layers required to configure and query hardware capabilities (RAM/VRAM/QoS).

- [x] **Schema Definition:** Replace the empty content of `tachyon-ui/gen/schemas/capabilities.json` with a strict JSON schema defining `HardwarePolicy` (RAM, VRAM, accelerators, QoS).
- [x] **UI Integration:** In the frontend (`tachyon-ui/src/`), create a dynamic form component that parses this JSON and integrates it into the Wasm deployment workflow.
- [x] **Tauri Data Binding:** Modify the Tauri commands in `tachyon-ui/src/main.rs` to accept and validate this new configuration block before forwarding it to `core-host`.
- [x] **MCP Telemetry Resource:** In `tachyon-mcp/src/main.rs`, expose an MCP resource that reads hardware state via the internal `core-host` or `system-faas-prom` APIs.
- [x] **MCP Validation Tool:** Implement an MCP tool that allows an LLM to submit a draft manifest and receive simulated approval or rejection from the Admission Control engine.
