# Proposal: Change 045 - Explicit Hardware WITs & QoS Scheduling

## Context
A generic `wasi-nn` interface hides the hardware reality from the developer. For maximum performance, FaaS developers must be able to target specific accelerators (GPU, NPU, TPU) at build time via specific WIT interfaces. Furthermore, with multiple FaaS instances competing for the same hardware queue, the host must enforce Quality of Service (QoS). A real-time user-facing inference must preempt a background data-processing task.

## Objective
1. Fork the generic `wasi-nn` into explicit Tachyon namespaces: `tachyon:accelerator/gpu`, `tachyon:accelerator/npu`, `tachyon:accelerator/tpu`, and `tachyon:accelerator/cpu`.
2. Introduce QoS classes (`RealTime`, `Standard`, `Batch`) in the `integrity.lock`.
3. Upgrade the Host's hardware queues from simple FIFO channels to Priority Queues.

## Scope
- Define 4 distinct WIT interfaces.
- Update the Wasmtime linker to expose these explicit interfaces to the guest.
- Implement a `PriorityQueue` (using `std::collections::BinaryHeap` or prioritized channels) for the GPU/NPU/TPU schedulers.

## Success Metrics
- A FaaS requesting the NPU WIT fails to instantiate gracefully at startup if no NPU is present, preventing silent performance degradation.
- When the GPU is saturated, a `RealTime` QoS request is processed before 10 pending `Batch` QoS requests, proving queue preemption.