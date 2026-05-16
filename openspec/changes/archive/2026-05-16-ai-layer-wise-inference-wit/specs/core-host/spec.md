# Technical Specification: tachyon:inference Architecture

## 1. The WIT Contract (`wit/ai/inference.wit`)
Define the zero-copy, handle-based layer manipulation contract.

```wit
package tachyon:mesh@1.1.0;

interface layer-execution {
    /// Opaque resource identifier representing a native Tensor on the host
    type tensor-handle = u32;
    type model-id = string;

    /// Instructs the host to slide the mmap window over specific safetensors blocks
    load-layer: func(model: model-id, layer-index: u32) -> result<_, string>;
    
    /// Evicts the specified layer weights from memory/VRAM
    unload-layer: func(model: model-id, layer-index: u32);

    /// Processes a vector of state pointers through the active layer weights
    forward-layer: func(
        model: model-id, 
        layer-index: u32, 
        inputs: list<tensor-handle>
    ) -> result<list<tensor-handle>, string>;

    /// Safely cleans up the tensor representation on the host side
    drop-tensor: func(handle: tensor-handle);
}

world inference-host {
    export layer-execution;
}
```

## 2. Conditional Compilation Architecture (`core-host/Cargo.toml`)
Isolate the structural overhead under an opt-in feature.

```toml
[features]
default = []
ai-inference = ["dep:candle-core", "dep:safetensors"]

[dependencies]
candle-core = { version = "0.8.0", optional = true }
safetensors = { version = "0.4.3", optional = true }
```

## 3. Host State Implementation (`core-host/src/ai_inference.rs`)
Encapsulate the state logic with standard conditional compilation fences.

```rust
#[cfg(feature = "ai-inference")]
use candle_core::Tensor;
use std::collections::HashMap;
use std::sync::Arc;

/// Dynamic runtime state wrapper mapped directly to the Wasmtime instance lifecycle
pub struct InstanceInferenceState {
    #[cfg(feature = "ai-inference")]
    pub active_tensors: HashMap<u32, Tensor>,
    pub next_handle: u32,
}

impl InstanceInferenceState {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "ai-inference")]
            active_tensors: HashMap::new(),
            next_handle: 0,
        }
    }
}

// When the feature is absent, compile empty stubs that return errors to the guest
#[cfg(not(feature = "ai-inference"))]
pub mod host_bindings {
    pub fn load_layer(_model: String, _idx: u32) -> Result<(), String> {
        Err("Inference feature not compiled in core-host".to_string())
    }
    // ... remaining stubs ...
}
```
