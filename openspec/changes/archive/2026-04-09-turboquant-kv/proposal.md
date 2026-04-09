# Proposal: TurboQuant KV Cache Integration for Candle (FFI)

## Objective
Integrate the TurboQuant KV Cache compression algorithm (PolarQuant + QJL) into our Rust-based Service Mesh inference engine using Hugging Face's `candle` framework. This will allow massive context windows (e.g., for Pacbase legacy code analysis) on consumer GPUs by reducing the KV cache footprint by up to 6x.

## Problem Statement
Standard float16/q8_0 KV caches cause Out-Of-Memory (OOM) errors on 8GB VRAM GPUs (like RTX 3070 Ti) when processing long context windows required for FaaS microservices extraction.

## Proposed Solution
We will wrap the highly optimized C++/CUDA kernels from the `llama-cpp-turboquant` fork and expose them to Rust via FFI, integrated as a `CustomOp` in Candle. 

To guarantee zero accuracy loss and high performance, the implementation MUST follow these three strict architectural rules:
1. **Asymmetric K/V Compression:** The keys ($K$) must remain in `q8_0` (or `f16`). Only the values ($V$) will be compressed using TurboQuant (2-bit/3-bit).
2. **Boundary Layers Protection:** The first 2 and last 2 layers of the LLM must bypass TurboQuant and keep $V$ in standard high-precision cache (`q8_0` or `f16`).
3. **Sparse V Decoding:** The C++ decompression kernel must accept an attention score threshold. If the attention weight for a token is near zero, the kernel must skip the memory read and decompression of that token's $V$ vector.

## Success Criteria
- Memory consumption for KV cache is significantly reduced.
- The model maintains coherence on long-context tasks (verified via Perplexity/Logit comparison).
- The FFI integration passes bit-for-bit equivalence tests against the reference C++ implementation.