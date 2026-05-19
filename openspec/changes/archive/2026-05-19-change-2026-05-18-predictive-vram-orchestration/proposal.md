# Proposal: Predictive VRAM Pre-Warming via Time-Series KV and Auth Telemetry

## Why
In a multi-tenant AI mesh, edge nodes dynamically swap tenant-specific LoRA adapters into the GPU. Loading a 500MB+ `.safetensors` file introduces a noticeable "Cold Start" latency for the user's first prompt. The `system-faas-model-broker` currently reacts to inference requests rather than anticipating them.

1. **UX Degradation:** Users experience 2 to 5 seconds of latency on their first session prompt while weights are mapped to VRAM.
2. **Resource Thrashing:** Constantly loading and unloading the same LoRAs for predictable users wastes PCIe bandwidth and power.
3. **Naive Predictive Loading:** Blindly pre-loading models via cron jobs risks starving the node's VRAM, blocking actual live inference requests.

## What Changes
Evolve the `system-faas-model-broker` into a proactive VRAM manager using a two-phased approach:
1. **Phase 1: Just-In-Time (JIT) Pre-Warming:** The broker subscribes to the `auth.session_started` CDC event. Upon login, the broker resolves the user's `x-tenant-id` and asynchronously loads their default LoRA adapter into VRAM *while* the user navigates the frontend UI.
2. **Phase 2: Probabilistic Eviction (Smart TTL):** Instead of cron-based loading, we use the Time-Series layer to compute usage probabilities. If the heuristics show a user typically sends bursts of prompts at this hour, the broker dynamically increases the VRAM Time-To-Live (TTL) for that specific LoRA, preventing premature eviction.
3. **Volatile Memory Tiers:** Any LoRA loaded via prediction or kept alive via extended TTL is tagged as `Priority::Volatile`. If a synchronous, live user prompt requires VRAM, the `core-host` will instantly overwrite volatile tensors to guarantee zero-downtime for active users.

## Impact
- **Zero-Latency AI:** Masks the 500MB transfer time behind the natural delay of human UI interaction (login -> typing).
- **VRAM Safety:** Retains Tachyon's strict hardware stability. Predictive models will never cause an Out-Of-Memory (OOM) failure for a live request.
