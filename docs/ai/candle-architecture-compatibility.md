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
| Qwen3-MoE | Supported | Recognized, no verified loader | Supported | Unsupported |
| Qwen 3.5 MoE ModelOpt/NVFP4 | Specialized runtime | Unsupported | Capability-gated | Specialized runtime |
| Gemma2 | Supported | Recognized, no verified loader | Supported | Unsupported |
| Gemma3 text | Supported | Recognized, no verified loader | Supported | Unsupported |
| Gemma3 multimodal | Unsupported | Unsupported | Unsupported | Unsupported |
| Phi3/Phi4-compatible | Supported | Recognized, no verified loader | Supported | Unsupported |
| DeepSeek V2 | Supported | Recognized, no verified loader | Supported | Unsupported |
| DeepSeek V3/R1 | Recognized, no verified loader | Recognized, no verified loader | Unsupported | Unsupported |

Recognized but unsupported combinations fail before weight loading with a typed
error naming the architecture, format, or execution mode. Gemma3 text requires
the pinned Candle fork revision `653ccb77`, which also exposes
`candle_transformers::models::qwen3_moe` for native Qwen3-MoE safetensors
generation.

Phi4 is accepted only when its configuration deserializes against the pinned
Phi3 Candle contract. DeepSeek V3/R1 stay fail-closed until the pinned Candle
fork exposes dedicated backends; Tachyon must not silently coerce those
architectures through the DeepSeek V2 loader.
