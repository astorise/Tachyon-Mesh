# Title: Native Semantic Relational Engine (Hexastore on Redb)

## Problem Statement
AI Swarm agents (like `viking-context` and `cognitive-gc`) require complex relational reasoning and multi-hop graph traversals. Relying on the guest WebAssembly to fetch and filter serialized adjacency lists from a flat Key-Value store creates a massive Inter-Process Communication (IPC) bottleneck and risks Wasm Out-Of-Memory (OOM) crashes.

## Objective
Implement a native Semantic Graph Store within Tachyon's `core-host` using the "Hexastore" approach on top of our existing `redb` storage engine.
1. **Component Model Integration:** Introduce the `tachyon:store/graph` WIT interface using the memory-safe `resource` semantic.
2. **Zero-Dependency Hexastore:** Avoid embedding heavy Triple-Store libraries (like CozoDB). Instead, create indexed tables (e.g., SPO, OSP) directly within `redb` to enable $O(\log N)$ semantic lookups.
3. **Host-Side Traversal:** Implement the `traverse` logic in Rust to process multi-hop queries natively, returning only the final result set to the Wasm guest to eliminate the IPC tax.