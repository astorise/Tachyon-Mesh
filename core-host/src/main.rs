#![deny(clippy::unwrap_used)]
#![recursion_limit = "256"]
// Items behind `#[cfg(feature = "experimental")]` and the bulk of the
// `ai_inference` module (layer-wise inference, predictive VRAM, semantic
// context flattening) are intentional v1.2 scaffolding that remain
// unwired until their OpenSpec proposals land. Compiling them under
// `--all-features` or `--features ai-inference` should not regress CI;
// default builds keep `-D dead_code` strict via RUSTFLAGS so production
// paths can't hide unused code.
#![cfg_attr(
    any(feature = "experimental", feature = "ai-inference-onnx"),
    allow(dead_code)
)]

#[cfg(feature = "ai-inference-onnx")]
mod ai_inference;
mod auth;
mod core_error;
mod data_events;
mod error;
mod iam_audit;
pub mod identity;
mod memory_governor;
mod mesh;
pub mod network;
#[cfg(feature = "rate-limit")]
mod rate_limit;
#[cfg(feature = "resiliency")]
mod resiliency;
pub mod runtime;
#[cfg(feature = "http3")]
mod server_h3;
pub mod state;
mod storage;
mod store;
mod system_storage;
pub mod telemetry;
mod tls_runtime;

mod host_core;
pub(crate) use host_core::*;

mod component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "faas-guest",
    });
}

mod system_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "system-faas-guest",
    });
}

mod udp_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "udp-faas-guest",
    });
}

#[cfg(feature = "websockets")]
mod websocket_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "websocket-faas-guest",
    });
}

mod background_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "background-system-faas",
    });
}

mod control_plane_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/tachyon.wit",
        world: "control-plane-faas",
    });
}

#[cfg(feature = "ai-inference-onnx")]
mod accelerator_component_bindings {
    wasmtime::component::bindgen!({
        path: "../wit/accelerator",
        world: "host",
    });
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    host_core::run().await
}
