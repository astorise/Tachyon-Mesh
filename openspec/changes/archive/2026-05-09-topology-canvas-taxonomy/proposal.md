# Title: Visual Node Taxonomy and Domain-Specific Canvas Overlays

## Problem Statement
A generic, monochromatic node-based GUI is insufficient for representing the complex, heterogeneous nature of the Tachyon-Mesh ecosystem. Operators must be able to visually distinguish between custom WASM workloads, System FaaS components, LLMs, storage volumes, external REST APIs, and message brokers at a glance. Furthermore, each node type requires completely different configuration parameters (e.g., an LLM needs a model name; a Cache needs a size allocation).

## Objective
Establish a strict visual taxonomy and property-editing architecture for the `<tachyon-topology-canvas>`.
1. Define distinct visual profiles (colors, icons, badges) for 8 core component types: Endpoint, System FaaS, Custom WASM, LLM, KV-Cache, Storage, Message Broker, and External Resource.
2. Implement contextual side-panels that dynamically render the correct configuration form based on the selected node's type.
3. Ensure the visual graph accurately serializes back into the composite `manifest.yaml` format.