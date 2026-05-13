# Title: Native Layer-Wise Inference Engine with Zero-Copy and Asynchronous Prefetching

## Problem Statement
Deploying large LLMs (e.g., 70B parameters) on Edge nodes with constrained VRAM (8-16 GB) natively results in Out-Of-Memory (OOM) errors. We must maintain Air-Gapped sovereignty, meaning cloud fallbacks are unacceptable. While standard layer-by-layer offloading solves the OOM issue, it introduces massive PCIe I/O bottlenecks.

## Objective
Implement an advanced, native Layer-Wise Inference engine within Tachyon's `core-host` using HuggingFace Candle.
1. **Zero-Copy Memory Mapping:** Map monolith `.safetensors` files directly to host RAM using `mmap` instead of chunking physical files.
2. **Prefill Batching:** Parallelize the attention mechanism during the prompt processing phase, streaming the model weights across the PCIe bus only *once* for the entire prompt to drastically reduce Time-To-First-Token (TTFT).
3. **Asynchronous Prefetching (Ring Buffer):** During autoregressive decoding, use concurrent compute/transfer streams (Double Buffering via CUDA streams) to overlap GPU computation of Layer $N$ with the PCIe Host-To-Device transfer of Layer $N+1$ and the Device-To-Host offloading of Layer $N-1$ and its KV-Cache.