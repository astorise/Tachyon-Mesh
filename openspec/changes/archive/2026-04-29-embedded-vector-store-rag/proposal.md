# Proposal: Embedded Vector Store for RAG (Retrieval-Augmented Generation)

## Context
To provide highly specialized AI per tenant (Multi-Tenant) without exhausting GPU VRAM (the critical bottleneck at the Edge), the RAG approach is the most resilient. Unlike fine-tuning (LoRA), RAG allows for injecting private and dynamic context directly into the FaaS prompt before inference. However, to maintain sub-millisecond latencies and support Air-Gapped environments, Tachyon Mesh cannot depend on external vector databases (like Pinecone) or heavy separate processes. It is necessary to integrate an ultra-fast, local similarity search capability.

## Objective
1. Extend `embedded-core-store` capabilities to support vector indexing and Approximate Nearest Neighbor (ANN) search in memory and on disk.
2. Expose these capabilities to WebAssembly FaaS workers via a new "Opt-In" WIT interface.
3. Maintain the "Zero-Cost Abstraction" principle: Wasm instances not using RAG incur no overhead.

## Scope
- Creation of a `wit/store/vector.wit` interface in the Wasm Component Model.
- Integration of a native Rust HNSW (Hierarchical Navigable Small World) algorithm into the `embedded-core-store` crate.
- Cryptographic isolation (Optional TDE) of vector indices to respect the Zero-Trust model.