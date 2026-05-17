# Proposal: BaaS Advanced Capabilities (CDC, RAG, CRDT)

## Why
With the foundational Tachyon BaaS Data Fabric in place, the mesh provides secure, distributed Key-Value storage and OLAP capabilities. To fully replace legacy cloud backends (like Firebase or Supabase) at the edge, Tachyon must natively support reactive user interfaces, long-term memory for local LLMs, and eventual consistency for offline-first mesh topologies.

1. **Inefficient UIs:** Clients polling RedDB for changes drain battery and network resources.
2. **Heavy AI Memory:** Running standard vector databases (Qdrant, Milvus) on edge nodes consumes too much RAM, hindering local RAG (Retrieval-Augmented Generation).
3. **Split-Brain Writes:** In an air-gapped or partitioned mesh, disconnected nodes will perform conflicting writes to the same keys, leading to data loss upon reconnection.

## What Changes
Leverage Tachyon's Wasm capabilities and WIT contracts to implement these features with zero structural overhead:
1. **Change Data Capture (CDC) & Pub/Sub:** Introduce a `data-events` WIT contract. The core-host broadcasts transaction logs to a `system-faas-cdc-broadcaster` which handles WebSocket/QUIC pushes and Biscuit-based RLS filtering.
2. **Ephemeral Vector Store:** Store embeddings as raw binary blocks in RustFS. Spawn an ephemeral `system-faas-vector-search` that `mmap`s the index, computes cosine similarity, yields results, and immediately terminates.
3. **CRDT Conflict Resolution:** Store vector clocks alongside K/V data. On reconnect, the core-host detects conflicts and invokes a user-defined Wasm function via a `conflict-resolution` WIT contract to execute domain-specific merge logic.

## Impact
- **Real-Time Edge:** Push-based architectures save bandwidth and CPU.
- **AI-Native:** Local LLMs (like Nebula/Pulsar) gain semantic memory without infrastructure bloat.
- **Offline-First:** The mesh becomes completely tolerant to network partitions, ensuring no data loss.
