# Proposal: Change 071 - Large Model Broker (Direct-to-Disk Streaming)

## Context
While the embedded asset registry (Change 070) perfectly handles small WebAssembly binaries and OCI manifests via memory-buffered KV storage, it is fatally unsuited for Large Language Models (LLMs) or heavy ONNX tensors. Uploading a multi-gigabyte `.gguf` file via standard HTTP POST buffering would result in Out-Of-Memory (OOM) crashes on the edge node. To support Air-Gapped AI deployments, Tachyon requires a dedicated subsystem capable of streaming massive files directly to disk without saturating RAM.

## Objective
1. Implement a chunked upload protocol (Multipart Upload) for AI models.
2. Develop a new system component, `system-faas-model-broker`, responsible for managing these direct-to-disk (D2D) streams.
3. Update `tachyon-client` and `tachyon-ui` to support slicing massive local files into 5MB chunks and pushing them sequentially with progress tracking.

## Scope
- Create `system-faas-model-broker` crate.
- Define a new multipart API on the `core-host` router (`/models/init`, `/models/upload`, `/models/commit`).
- Isolate the storage path for models (e.g., a dedicated `models/` directory) away from the transactional `redb` database.