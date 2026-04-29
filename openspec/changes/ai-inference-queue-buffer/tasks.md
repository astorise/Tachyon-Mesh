# Implementation Tasks

## Phase 1: Queue Integration
- [x] Ensure `system-faas-buffer` exposes standard queue operations (`push`, `pop`, `ack`) via IPC.
- [ ] Update the `core-host` API router to intercept AI generation requests and route them to the buffer.
- [ ] Implement the `202 Accepted` response logic returning a generated `job_id`.

## Phase 2: Worker Logic
- [ ] Modify `system-faas-ai-inference` to act as a background worker rather than a synchronous web handler.
- [ ] Implement the loop that pulls jobs from `system-faas-buffer`.
- [ ] Ensure proper error handling: if the inference traps or fails, the job should either be retried (up to max attempts) or moved to a Dead Letter Queue (DLQ).

## Phase 3: Result Delivery
- [ ] Implement a status endpoint (`GET /api/v1/jobs/:id`) to retrieve generated outputs.
- [ ] (Optional but recommended) Hook the completion event into the existing WebSocket module for real-time token streaming back to the client.

## Phase 4: Validation
- [ ] **Burst Test:** Send 50 concurrent large LLM inference requests to the router.
- [ ] Verify that the GPU processes them sequentially (one or a few at a time depending on batching config) without crashing.
- [ ] Verify that all 50 clients eventually receive their generated text.
