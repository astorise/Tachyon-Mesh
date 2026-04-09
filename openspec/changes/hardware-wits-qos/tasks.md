# Tasks: Change 045 Implementation

**Agent Instruction:** Implement explicit hardware WITs and Priority Queues for QoS. Do not use nested code blocks in your outputs.

## [TASK-1] Define and Link Explicit WITs
- [ ] Create 4 separate `.wit` files for CPU, GPU, NPU, and TPU under the `tachyon:accelerator` namespace.
- [ ] In the `core-host` Wasmtime linker setup, conditionally link these interfaces based on detected host hardware. If the node has no TPU, do not link the TPU interface (causing an instant instantiation failure for FaaS that require it).

## [TASK-2] Parse QoS and Tag Requests
- [ ] Update the `integrity.lock` parser to accept `qos` as an enum: `RealTime` (Score 100), `Standard` (Score 50), and `Batch` (Score 10).
- [ ] Modify the host's `compute` hook for each WIT to attach the FaaS's assigned QoS score to the `InferenceRequest` before sending it to the hardware queue.

## [TASK-3] Implement Priority Queues with Ageing
- [ ] Replace the standard `tokio::mpsc` channels in the hardware schedulers with a thread-safe Priority Queue (e.g., using a `Mutex<BinaryHeap>` paired with a `tokio::sync::Notify`).
- [ ] Implement the `Ord` trait for requests to sort primarily by `qos_score`, and secondarily by `timestamp` (FIFO for same priority).
- [ ] Implement Ageing: When the scheduler pops a batch of tasks, increment the `qos_score` of the remaining tasks in the heap to prevent starvation.

## Validation Step
- [ ] Configure `gpu-bot` (RealTime) and `gpu-batch` (Batch).
- [ ] Flood the GPU with 50 `gpu-batch` requests (which will take several seconds to process).
- [ ] Immediately fire 1 `gpu-bot` request.
- [ ] Verify via host logs that the `gpu-bot` request is pushed to the front of the queue and processed in the very next batch, preempting the remaining `gpu-batch` requests.
