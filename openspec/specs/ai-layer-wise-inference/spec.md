# ai-layer-wise-inference Specification

## Purpose
Layer-wise streaming inference: O(1) VRAM footprint via memory-mapped weight loading, prefill batching, and ring-buffer decode pipeline with KV-Cache host-RAM paging.
## Requirements
### Requirement: Layer-wise inference and pipeline behavior MUST be specified
The project SHALL update `wit/ai/inference.wit` and the core AI inference implementation so layer-wise streaming inference can maintain an O(1) VRAM footprint via memory-mapped weight loading, prefill batching, asynchronous decode pipelining, and KV-cache host-RAM paging.

#### Scenario: WIT contract exposes memory profiles
- **WHEN** `wit/ai/inference.wit` is inspected
- **THEN** it defines an inference options contract that can select `performance` or `layer-wise-streaming` memory profiles

#### Scenario: Layer-wise streaming avoids full-model VRAM residency
- **WHEN** `memory-profile` is `layer-wise-streaming`
- **THEN** the host maps `.safetensors` weights in host RAM and only loads the active layer weights to VRAM
- **AND** KV-cache state is paged outside VRAM between layer executions

#### Scenario: Decode overlaps copy and compute work
- **WHEN** autoregressive decode runs with layer-wise streaming
- **THEN** the host overlaps layer compute with weight prefetch and KV-cache offload where the accelerator backend supports separate streams

#### WIT Contract Modifications
Codex must update `wit/ai/inference.wit`:
```wit
package tachyon:ai@1.1.0;

interface inference {
    enum memory-profile {
        /// Default: load all tensors to VRAM. High speed, high OOM risk.
        performance,
        /// Streaming: offloads layers and KV-cache to NVMe/Host RAM. Low VRAM footprint.
        layer-wise-streaming,
    }

    record inference-options {
        temperature: f32,
        max-tokens: u32,
        profile: memory-profile,
    }

    generate: func(model: string, prompt: string, options: inference-options) -> result<string, string>;
}
```

#### Zero-Copy Tensor Mapping (`ai_inference.rs`)
When `memory-profile` is `layer-wise-streaming`:
- Codex must NOT instantiate the full model architecture directly on the `Device::new_cuda(0)`.
- Codex must use the `memmap2` crate to map the `.safetensors` file into host RAM (`Device::Cpu`).
- Initialize a `ModelWeights` struct that holds the mapped file pointers, but does not allocate VRAM.

#### Prefill Batching Logic (Phase 1)
During the initial prompt tokenization:
- **Rule:** Do NOT process the prompt one token at a time.
- Transfer Layer 0 weights to GPU. Process the entire sequence of prompt tokens through Layer 0.
- Store the resulting Hidden States (Activations) in GPU memory.
- Store the KV-Cache for Layer 0 in Host RAM (Swap out).
- Drop Layer 0 weights from GPU. Transfer Layer 1 weights to GPU.
- Process the Hidden States through Layer 1. Repeat until the final model layer generates the first output token (TTFT).

#### Asynchronous Pipelining (Phase 2 - Decode)
For the autoregressive token generation phase, Codex must implement an overlapping I/O and Compute strategy.
- Determine the maximum number of layers that safely fit in the user's VRAM (e.g., $K=4$).
- Establish two distinct execution streams on the device (if using CUDARC/Candle integration: one Compute Stream, one Copy Stream).
- **The Pipeline Loop for Layer $N$:**
  1. **Compute Stream:** Executes the forward pass for Token $T$ on Layer $N$.
  2. **Copy Stream (Host-to-Device):** Concurrently loads the weights of Layer $N+1$ from the mapped CPU memory into a pre-allocated GPU buffer.
  3. **Copy Stream (Device-to-Host):** Concurrently offloads the updated KV-Cache slice of Layer $N$ to Host RAM and drops Layer $N-1$ weights from VRAM.
- Synchronize streams using CUDA events before proceeding to compute Layer $N+1$.

#### KV-Cache Management
To ensure a strictly constant $O(1)$ VRAM footprint regardless of context length:
- The KV-Cache cannot grow infinitely on the GPU.
- Allocate a fixed-size buffer on the GPU for the *current* layer's KV-Cache.
- After Layer $N$ computes, its KV-Cache must be appended/swapped to a Host CPU buffer (or dispatched to `tachyon:ai/kv-partition` V2 if integrating with the swarm context memory). 
- When computing Layer $N$ for the next token, page back its specific KV-Cache slice from CPU to GPU.

### Requirement: Layer-wise streaming MUST preserve NVFP4 tensor structure
Layer-wise streaming for ModelOpt/NVFP4 checkpoints SHALL map safetensors shards by tensor name and vend typed per-layer quantized components instead of partitioning raw bytes into equal `f32` slices.

#### Scenario: Active layer loads typed NVFP4 components
- **WHEN** `memory-profile` is `layer-wise-streaming`
- **AND** the active layer contains ModelOpt/NVFP4 linear operators
- **THEN** the loader maps the packed weights, block scales, tensor scales, and any BF16 tensors required for that layer
- **AND** it transfers or dequantizes only the active layer's required components according to the selected backend

#### Scenario: Sharded tensor index drives layer mapping
- **WHEN** a ModelOpt/NVFP4 checkpoint uses multiple safetensors shards
- **THEN** the layer-wise loader resolves each tensor through `model.safetensors.index.json`
- **AND** it never assumes all weights for a layer are contiguous in a single equal-sized byte range

### Requirement: Layer-wise NVFP4 execution MUST keep memory-profile semantics
The ModelOpt/NVFP4 layer-wise runtime SHALL preserve the existing performance and layer-wise-streaming memory profile behavior while accounting for packed quantized storage and fallback dequantization.

#### Scenario: Layer-wise streaming avoids full packed-model residency on accelerator
- **WHEN** a ModelOpt/NVFP4 model runs with `layer-wise-streaming`
- **THEN** the runtime does not load all model layers into accelerator memory at once
- **AND** it pages KV cache and layer weights according to the existing layer-wise streaming contract

#### Scenario: Fallback dequantization respects memory limits
- **WHEN** native NVFP4 kernels are unavailable under `layer-wise-streaming`
- **THEN** the runtime may dequantize only the active layer or configured layer window
- **AND** it rejects execution if fallback dequantization would require full-model accelerator residency
