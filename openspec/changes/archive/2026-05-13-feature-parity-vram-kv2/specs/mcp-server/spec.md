# Technical Specification: MCP VRAM Intelligence

## 1. Enhance Hardware Status (`tachyon-mcp/src/main.rs`)
Update the existing `tachyon_hardware_status` tool to ensure it explicitly returns the VRAM metrics required by agents to make routing decisions.

```rust
// In the MCP handler for hardware status
let status = tachyon_client::read_local_hardware_status_async().await?;

// Ensure the JSON response serializes the GPU topological data
/*
{
  "cpu_usage": 45.2,
  "ram_free_mb": 12048,
  "gpus": [
    {
      "id": "gpu-0",
      "model": "RTX 4090",
      "vram_total_mb": 24576,
      "vram_used_mb": 18432,
      "compute_utilization": 88.5
    }
  ]
}
*/
```

## 2. VRAM Tooling
If the agent needs to force a deployment to a specific GPU, ensure the `tachyon_apply_manifest` JSON Schema (from the P0 patch) includes the newly supported `nodeSelector` and `vram_requirements` fields so the agent knows it can use them.