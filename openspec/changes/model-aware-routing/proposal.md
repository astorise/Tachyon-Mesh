# Proposal: Change 047 - Model-Aware Routing & Hot-VRAM Affinity

## Context
In a multi-node, heterogeneous cluster (Change 046), overflowing a RealTime AI request to a node with a "free GPU" is a fatal error if that GPU does not currently have the required LLM weights loaded in its VRAM. Loading a 40GB model from NVMe to VRAM takes 15-30 seconds, entirely defeating the purpose of RealTime QoS. The Mesh must be aware of "Data Locality" and "Hot VRAM State" to make intelligent steering decisions.

## Objective
1. The `core-host` must expose exactly which models are currently "Hot" (loaded in VRAM).
2. The `system-faas-gossip` must broadcast this "Hot-VRAM State" to the cluster.
3. The Mesh Router must enforce "Model Affinity": a RealTime request can ONLY overflow to a peer node if that peer already has the target model loaded in its VRAM.

## Scope
- Extend `wasi:tachyon/telemetry` to list active model aliases.
- Update the Gossip protocol payload to map `Node_IP -> Free_GPU_Slots -> [Loaded_Models]`.
- Implement Affinity routing logic in the HTTP dispatcher.

## Success Metrics
- A RealTime request for "llama-3" on a saturated Node A will overflow to Node B (busy, but has llama-3 loaded) rather than Node C (completely idle, but has mistral loaded), resulting in a <100ms response time instead of a 30-second cold start.