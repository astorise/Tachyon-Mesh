# Implementation Tasks

- [x] **Task 1: VRAM UI Component**
  - Update `TachyonHardwarePanel.ts` to parse GPU VRAM data from the websocket telemetry stream.
  - Render progress bars for each detected GPU in the cluster.

- [x] **Task 2: KV Explorer UI**
  - Update `TachyonStoragePanel.ts` (or create `TachyonKVExplorer.ts`).
  - Implement a data grid using the `tachyon_client` (via Tauri IPC) to list namespaces and keys.
  - Add a delete button to clear specific keys manually.

- [x] **Task 3: MCP Hardware Payload**
  - Verify that `core-host` exposes the GPU array in its hardware telemetry endpoint.
  - Update `tachyon-mcp` to ensure this specific data block is passed unaltered to the LLM agent when `tachyon_hardware_status` is called.

- [x] **Task 4: Schema Alignment for VRAM**
  - Ensure the `Manifest` schema in `core-host` generated via `schemars` accurately describes the `vram_mb` and `gpu_affinity` configuration fields for FaaS workloads.
