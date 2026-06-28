# Candle architecture compatibility

This matrix tracks native `core-host` text-generation support for
[GitHub issue #228](https://github.com/astorise/Tachyon-Mesh/issues/228).
Architecture selection uses checkpoint metadata, never the model alias or
directory name.

| Architecture | HF safetensors | GGUF | Single device | Parallel modes |
| --- | --- | --- | --- | --- |
| Llama | Supported | Supported | Supported | Tensor and pipeline |
| Mixtral | Expert-parallel only | Unsupported | Unsupported | Expert |
| Qwen2 dense | Supported | Recognized, no verified loader | Supported | Unsupported |
| Qwen3 dense | Supported | Recognized, no verified loader | Supported | Unsupported |
| Qwen 3.5 MoE ModelOpt/NVFP4 | Specialized runtime | Unsupported | Capability-gated | Specialized runtime |
| Gemma2 | Supported | Recognized, no verified loader | Supported | Unsupported |
| Gemma3 text | Supported | Recognized, no verified loader | Supported | Unsupported |
| Gemma3 multimodal | Unsupported | Unsupported | Unsupported | Unsupported |
| Phi3/Phi4-compatible | Supported | Recognized, no verified loader | Supported | Unsupported |
| DeepSeek V2/V3/R1-compatible | Supported | Recognized where metadata maps cleanly | Supported | Unsupported |

Recognized but unsupported combinations fail before weight loading with a typed
error naming the architecture, format, or execution mode. Gemma3 text requires
the pinned Candle fork revision `2ba71712`, which makes transposed K/V tensors
contiguous before appending them to the KV cache and carries the downstream
weight-quantization kernel work from `astorise/candle` for GPTQ/Marlin, AWQ,
and block-wise FP8 proposed upstream in `huggingface/candle#3650`.

Phi4 and DeepSeek V3/R1 are accepted only when their configuration deserializes
against the pinned Phi3 or DeepSeek V2 Candle contracts respectively. Semantic
variants with different required fields or tensor layouts fail during config or
weight validation rather than being silently coerced.
