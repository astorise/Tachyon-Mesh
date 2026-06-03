# Candle LLM Runtime

Tachyon's `core-host --features ai-inference` build supports a real Candle-backed
LLM path for local text-generation model bindings. This path is separate from
legacy Candle ONNX/WASI-NN graph loading and from the ModelOpt/NVFP4 loader.

## Supported Fixture Family

The first supported family is `TachyonTinyCausalLM`, identified by:

```json
{
  "model_type": "tachyon_tiny_causal_lm",
  "architectures": ["TachyonTinyCausalLM"]
}
```

The model directory must contain:

- `config.json`
- `tokenizer.json`
- `model.safetensors`

`model.safetensors` must contain an F32 tensor named `next_token_logits` with
shape `[vocab_size]`. The runtime loads the tokenizer with `tokenizers`, loads
the tensor through Candle safetensors support, and performs deterministic greedy
selection with Candle `argmax`.

This fixture is intentionally small so CI can validate real Candle execution
without downloading external artifacts.

## Prompt Input

The first `U8` inference tensor is interpreted as either plain UTF-8 prompt text
or a JSON generation request.

Plain prompt:

```text
hello
```

JSON request:

```json
{
  "prompt": "hello",
  "max_new_tokens": 1,
  "temperature": 0.0,
  "seed": 7
}
```

Default generation is deterministic. Requests are bounded by prompt byte count,
prompt token count, batch size, and the host max-new-token cap.

## Mock and Unsupported Bindings

Mock output is only available through explicit mock bindings such as:

```text
mock:llama3
```

Non-mock model bindings that are not supported Candle LLM directories, ONNX
guest-loaded graphs, or ModelOpt/NVFP4 directories fail during initialization
with an actionable error. They do not fall back to `MOCK_LLM_RESPONSE`.

## NVFP4 Boundary

ModelOpt/NVFP4 remains a separate loader and kernel capability. A detected
NVFP4 directory is classified as non-mock and still returns the existing
unsupported-execution error until a complete architecture runtime is wired.
