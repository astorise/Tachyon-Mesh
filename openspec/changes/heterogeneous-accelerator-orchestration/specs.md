# Specifications: 4-Way Hardware Dispatching

## 1. Universal Hardware Mapping
The `integrity.lock` defines the "Preferred Hardware". Tachyon will fail to start if the required hardware is missing, unless a `fallback: "cpu"` is specified.

    {
        "targets": [
            {
                "name": "multimodal-bot",
                "models": [
                    { "alias": "brain-llm", "path": "llama3.gguf", "device": "gpu" },
                    { "alias": "ears-stt", "path": "whisper.xml", "device": "npu" },
                    { "alias": "vision-ocr", "path": "donut.tflite", "device": "tpu" },
                    { "alias": "classifier", "path": "xgboost.onnx", "device": "cpu" }
                ]
            }
        ]
    }

## 2. Backend Implementation Matrix
- **CPU:** High-performance fallback using `ONNX Runtime` or `OpenVINO (CPU)`.
- **GPU (Candle):** Pure Rust + CUDA/Metal for LLMs (Change 042).
- **NPU:** `OpenVINO` (Intel), `CoreML` (Apple), or `Qualcomm SDK`.
- **TPU:** Integration via `LibTPU` or `XLA` bindings for Google Cloud TPU or Coral Edge TPU.

## 3. The Dispatcher (The "Brain")
Each device type has its own **Work Queue** (Async Channel). 
- The Host identifies the target device in the `wasi-nn` call.
- It pushes the request into the specific Device Queue.
- This ensures that hardware-level blocking (e.g., CUDA synchronization) never blocks the CPU or NPU execution threads.