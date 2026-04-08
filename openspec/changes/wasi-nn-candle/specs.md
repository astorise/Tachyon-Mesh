# Specifications: WASI-NN & Candle Architecture

## 1. Schema Update (`integrity.lock`)
We extend the configuration to define the model binding for the host. The FaaS relies on the host having the model ready.

    {
        "targets": [
            {
                "name": "ai-assistant",
                "module": "llm-worker.wasm",
                "role": "user",
                "scale": {
                    "min": 0,
                    "max": 100
                },
                "models": [
                    {
                        "alias": "llama3",
                        "path": "/models/llama3-8b.gguf",
                        "device": "cuda"
                    }
                ]
            }
        ]
    }

## 2. The Host GPU Scheduler (Candle)
- **Initialization:** Upon reading the config, the host uses `candle_core` to load the `.gguf` or `.safetensors` file into the specified device's VRAM. This happens once.
- **The Queue:** The host spawns a `Batcher` task. It listens on an async channel for `InferenceRequest` structs containing the prompt and a response channel.
- **Continuous Batching:** 1. The Batcher collects up to `BATCH_SIZE` requests (e.g., 32) from the queue within a short time window (e.g., 50ms).
  2. It tokenizes all prompts, padding them to the same length.
  3. It executes `model.forward(batched_tensors)`.
  4. It extracts the generated tokens, checks for stop sequences, and sends the individual results back through the respective response channels.

## 3. The WASI-NN Bridge
When the User FaaS executes `wasi_nn::compute()`:
1. The Wasmtime host intercepts the call.
2. It packages the input tensor (the prompt) into an `InferenceRequest`.
3. It sends the request to the Batcher channel and asynchronously `.await`s the response, parking the FaaS executor thread (yielding CPU).
4. Once the Batcher replies, the host writes the output tensor back into the WASM memory space and resumes the FaaS.