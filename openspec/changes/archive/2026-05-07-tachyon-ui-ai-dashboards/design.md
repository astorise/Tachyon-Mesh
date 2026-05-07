# Design: AI & Hardware Dashboards

The AI and hardware dashboards use the existing `TachyonConfigDashboard` base class so styling, feedback rendering, and GSAP entrance behavior stay consistent with routing and resilience panels.

The frontend adds two route-registered web components:

- `<tachyon-ai-panel>` manages LoRA mode, KV cache size, and TDE key input.
- `<tachyon-hardware-panel>` manages accelerator selection and XDP offload policy.

Both panels submit through the existing `apply_configuration` Tauri command. The Rust side adds a strict `config-ai` contract using `serde(deny_unknown_fields)`, enums for known modes, bounded KV cache validation, and non-empty key validation.
