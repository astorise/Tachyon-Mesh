# Specifications: Capability Routing

## 1. The Capability Tags
Capabilities are strings (or enum variants) representing what the node can physically do. They are automatically injected at compile-time by the Cargo Workspace features.

Examples:
- `core:wasi` (Can run WebAssembly)
- `legacy:oci` (Can run Docker/Youki containers - Linux only)
- `accel:cuda` (Has Nvidia GPU + Candle compiled)
- `accel:openvino` (Has Intel NPU compiled)
- `net:layer4` (Can do raw TCP/UDP bridging)

## 2. Gossip Payload Update
The Gossip state broadcasted every second now includes the static capabilities.

    {
        "node_id": "tachyon-edge-99",
        "ip": "10.0.0.5",
        "load_score": 42,
        "capabilities": ["core:wasi", "net:layer4"] // This node cannot run Docker or AI
    }

## 3. Target Requirements (`integrity.lock`)
When defining a target, the developer can explicitly request capabilities. If omitted, the system defaults to `["core:wasi"]`.

    {
        "targets": [
            {
                "name": "ai-summarizer",
                "module": "llm.wasm",
                "requires": ["core:wasi", "accel:cuda"]
            },
            {
                "name": "legacy-db",
                "type": "oci-container",
                "requires": ["legacy:oci"]
            }
        ]
    }

## 4. The Routing Algorithm (Filter-Then-Score)
When a request arrives for `ai-summarizer`:
1. **Filter:** The local router drops all peers from the Gossip table that do NOT possess both `core:wasi` AND `accel:cuda`.
2. **Score:** Among the remaining capable nodes, apply the QoS logic (Pick the one with the lowest load, or Hot-VRAM affinity).
3. **Route:** Forward the execution.