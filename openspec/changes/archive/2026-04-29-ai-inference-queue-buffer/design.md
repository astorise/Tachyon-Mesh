# Design: Asynchronous Inference Pipeline

## 1. Request Ingestion
Modify the HTTP routing middleware for AI endpoints (e.g., `/api/v1/generate`).
- **Action:** Instead of calling `ai_inference.generate()`, serialize the HTTP payload and send it to `system-faas-buffer.push()`.
- **Response:** Return `HTTP 202 Accepted` with a JSON body: `{ "job_id": "<uuid>", "status": "queued" }`.

## 2. Worker Execution (`core-host` or `system-faas-ai-inference`)
The inference Wasm module needs a polling loop or event subscriber.
- **Workflow:**
  1. Fetch `next_job` from `system-faas-buffer`.
  2. If a job exists, acquire the hardware lock (`accelerator-gpu.wit`).
  3. Perform the inference (e.g., using the newly integrated native Rust `turboquant` cache).
  4. Write the result to a temporary key-value store (or trigger a websocket event).
  5. Acknowledge and remove the job from the buffer.

## 3. Client Data Retrieval
- Option A (Polling): The client calls `/api/v1/generate/status/:job_id` to check if the result is ready.
- Option B (Streaming/Push): The host emits a `tachyon.events.ai.completed` event, and the `system-faas-websocket` module pushes the generated tokens directly to the connected client.