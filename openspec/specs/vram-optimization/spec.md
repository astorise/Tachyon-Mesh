# vram-optimization Specification

## Purpose
VRAM-aware routing, PCIe host-RAM offloading, and semantic context flattening for AI inference at the edge.

## Requirements

### Requirement: Local KV-cache tiering
The memory governor SHALL handle VRAM exhaustion by mapping new KV-cache tensor shards to pinned host memory when local VRAM allocation would exceed 90%, and MUST NOT fall back to disk or NVMe swapping.

#### Scenario: KV-cache spills to pinned host memory
- **GIVEN** a KV-cache allocation would push local VRAM usage above 90%
- **WHEN** the memory governor allocates new tensor shards
- **THEN** it maps those shards to pinned host memory over PCIe
- **AND** it does not allocate disk-backed swap for inference state

### Requirement: VRAM-aware load balancing
The L7 router SHALL query `telemetry::TelemetrySnapshot` for candidate node `vram_utilization`, penalize nodes above 80%, and queue inference requests locally when all candidates exceed 90%.

#### Scenario: Saturated candidates are queued
- **GIVEN** every candidate node reports VRAM utilization above 90%
- **WHEN** an inference request is routed
- **THEN** the router places the request into a bounded local await queue
- **AND** it avoids immediate HTTP 429 responses caused only by temporary VRAM saturation

### Requirement: Semantic marker inlining
The feature flattener SHALL identify semantic markers such as conversational turn IDs and system prompt boundaries and inline them into tokenized chunk metadata instead of traversing generic JSON structures.

#### Scenario: Conversational sequence preserves cache locality
- **GIVEN** an OpenAI-style conversational payload contains ordered message boundaries
- **WHEN** the feature flattener prepares tokenized chunk metadata
- **THEN** semantic markers are represented directly in the metadata
- **AND** contiguous logical sequences receive adjacent KV-cache keys
