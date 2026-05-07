# Proposal: AI & Hardware Orchestration Dashboards

## Why
Managing edge AI requires operator control over hardware accelerators, LoRA multiplexing, and KV cache distribution from the Tachyon UI shell.

## What Changes
- Add `<tachyon-ai-panel>` for LoRA mode, KV cache sizing, TDE key capture, and AI control plane sync.
- Add `<tachyon-hardware-panel>` for NPU, TPU, and GPU selection with eBPF XDP offload control.
- Add backend validation for strict `config-ai` payloads.

## Security
TDE key material is captured through a password-masked input and is only sent through the existing Tauri command path for backend validation.
