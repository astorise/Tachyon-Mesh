# Proposal: Atomic Downloads for Model Broker

## Context
The `system-faas-model-broker` is responsible for streaming large AI models (often 5GB to 40GB+ in GGUF format) directly to the host's persistent storage. Currently, if an HTTP/3 network stream is interrupted or a client aborts the upload/download, the partially written file remains on the disk indefinitely. Over time, these corrupted fragments will completely saturate the limited storage capacity of Edge and Air-Gapped nodes. Furthermore, the AI inference engine might erroneously attempt to load an incomplete model, causing a panic.

## Proposed Solution
We will implement an **Atomic Download Pattern**:
1. All active streams will write data to a temporary file appended with the `.part` extension (e.g., `llama3-8b.gguf.part`).
2. Only upon the successful completion of the entire stream will the broker perform an atomic filesystem operation (`fs::rename`) to remove the `.part` extension.
3. If an error occurs during the stream, the broker will attempt an immediate cleanup of the `.part` file.
4. As a fallback, any orphaned `.part` files left behind by hard crashes will be automatically swept by the newly resilient `system-faas-gc` based on its TTL configuration.

## Objectives
- Prevent storage exhaustion caused by aborted or flaky network transfers.
- Guarantee file integrity: a file without a `.part` extension is mathematically guaranteed to be fully downloaded.