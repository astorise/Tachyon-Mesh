# Proposal: AI Inference Request Buffering

## Context
AI model inference (especially LLMs) is highly bound by hardware accelerator capacity (VRAM and Compute Cores). In an Edge environment, a sudden burst of concurrent inference requests can easily exceed the local GPU/NPU capacity, leading to `ResourceExhausted` errors, timeouts, or host crashes. Dynamically falling back to a CPU is not viable due to the extreme latency degradation.

## Proposed Solution
Instead of synchronous request-response processing, we will adopt an **Asynchronous Queue Pattern**:
1. When an AI generation request arrives, the API gateway routes it to a message broker/buffer FaaS (e.g., `system-faas-buffer`).
2. The client immediately receives a `202 Accepted` response containing a `job_id`.
3. The `system-faas-ai-inference` module acts as a worker. It polls the buffer FaaS or listens to an event trigger, pulls the next request, and executes it sequentially using the available hardware accelerator.
4. The client can poll a status endpoint or receive the final result via the existing `system-faas-websocket` infrastructure once the generation is complete.

## Objectives
- Prevent hardware saturation and Out-of-Memory (OOM) errors on GPUs.
- Guarantee that no valid inference requests are dropped during traffic bursts.
- Provide a responsive UX by immediately acknowledging request receipt.