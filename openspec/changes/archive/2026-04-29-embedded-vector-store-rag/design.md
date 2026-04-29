# Design: Embedded Vector Store Architecture

## 1. Zero-Cost Abstraction (Opt-In via WIT)
We define a new Wasm contract: `tachyon:store/vector`.
- If a Wasm FaaS module imports this interface, the host (Wasmtime) injects handlers to the Rust HNSW engine.
- Embedding inference (transforming text to vectors) ideally runs on the CPU or NPU via small models (e.g., `all-MiniLM-L6-v2`) allocated by `wasi-nn`, leaving the main GPU dedicated to foundation model generation.

## 2. HNSW Integration & Storage
Unlike the transactional KV Store (`redb`), vectors require a specialized graph index.
- Vector indices will be saved under `tachyon_data/vectors/`.
- Logic will be embedded directly into the router (`core-host` crate via `embedded-core-store`) using a lightweight, safe implementation (e.g., `hnswlib-rs` or a native HNSW structure).

## 3. The RAG FaaS Workflow (I/O Isolation)
The complete pipeline for a RAG execution on Tachyon Mesh:
1. **FaaS execution:** HTTP/3 request arrives, Wasm FaaS is awakened.
2. **Embedding (CPU/NPU):** FaaS calls `wasi-nn` with a pre-loaded embedding model ID to vectorize the query.
3. **Similarity Search (CPU/Disk):** FaaS calls the new Host API `vector::search(embedding, limit)`. The Host queries the tenant's local index and returns relevant text chunks.
4. **Prompt Construction:** Wasm code concatenates chunks to the initial prompt.
5. **LLM Inference (GPU):** Augmented prompt is sent to `system-faas-buffer` for processing by the foundation model (Candle) without hardware Context Switching.

## 4. Governance and Air-Gapped
This local architecture allows managing tenant "Knowledge" via simple CRUD calls (Upsert/Delete of text chunks), facilitating GDPR compliance and updates without requiring any re-training.