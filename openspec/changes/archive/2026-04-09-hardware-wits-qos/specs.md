# Specifications: Explicit WITs and QoS

## 1. The Explicit WIT Interfaces
Instead of standard `wasi-nn`, developers import specific Tachyon interfaces in their Rust/Go code. This breaks generic portability but enforces hardware guarantees at build time.

Example `tachyon:accelerator/gpu`:

    interface gpu-inference {
        load-model: func(name: string) -> result<u32, string>;
        compute: func(model-id: u32, prompt: string) -> result<string, string>;
    }

*(The host exposes similar explicit interfaces for NPU, TPU, and CPU, potentially exposing hardware-specific limits like VRAM pinning constraints).*

## 2. QoS Configuration (`integrity.lock`)
The QoS level is defined per target, granting strict priority in the hardware execution queues.

    {
        "targets": [
            {
                "name": "live-chatbot",
                "module": "gpu-bot.wasm",
                "qos": "RealTime" 
            },
            {
                "name": "nightly-summarizer",
                "module": "gpu-batch.wasm",
                "qos": "Batch" 
            }
        ]
    }

*Note: If omitted, the `qos` defaults to `Standard`.*

## 3. Priority Schedulers
The `BatchScheduler` for each hardware queue is upgraded to support preemption:
- Incoming requests to the hardware (e.g., GPU) are wrapped in a `PrioritizedTask` struct: `{ qos_score: u8, timestamp: u64, payload: Tensor }`.
- Scoring definition: `RealTime` = 100, `Standard` = 50, `Batch` = 10.
- The Scheduler pulls tasks from a thread-safe Priority Queue (`BinaryHeap`). It always pops the task with the highest `qos_score` first.
- **Ageing Mechanism:** To prevent resource starvation (where `Batch` tasks never execute because `RealTime` tasks keep arriving), the `qos_score` of waiting tasks increases mathematically for every millisecond they spend in the queue. Eventually, a `Batch` task's score will exceed 100, guaranteeing its execution.