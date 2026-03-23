use anyhow::{anyhow, Result};
use std::path::PathBuf;
use wasmtime::{Engine, Linker, Module, Store, TypedFunc};
use wasmtime_wasi::p1::{self, WasiP1Ctx};
use wasmtime_wasi::WasiCtxBuilder;

struct HostState {
    wasi: WasiP1Ctx,
}

impl HostState {
    fn new() -> Self {
        let wasi = WasiCtxBuilder::new().inherit_stdio().build_p1();
        Self { wasi }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    run().await
}

async fn run() -> Result<()> {
    let engine = Engine::default();
    let mut linker = Linker::new(&engine);
    p1::add_to_linker_async(&mut linker, |state: &mut HostState| &mut state.wasi)
        .map_err(|error| anyhow!("failed to add WASI preview1 functions to linker: {error}"))?;

    let mut store = Store::new(&engine, HostState::new());
    let module_path = resolve_guest_module_path()?;
    let module = Module::from_file(&engine, &module_path).map_err(|error| {
        anyhow!(
            "failed to load guest module from {}: {error}",
            module_path.display()
        )
    })?;
    let instance = linker
        .instantiate_async(&mut store, &module)
        .await
        .map_err(|error| anyhow!("failed to instantiate guest module: {error}"))?;
    let faas_entry: TypedFunc<(), ()> = instance
        .get_typed_func(&mut store, "faas_entry")
        .map_err(|error| anyhow!("failed to resolve exported function `faas_entry`: {error}"))?;

    faas_entry
        .call_async(&mut store, ())
        .await
        .map_err(|error| anyhow!("guest function `faas_entry` trapped: {error}"))?;

    Ok(())
}

fn resolve_guest_module_path() -> Result<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = [
        manifest_dir.join("../target/wasm32-wasip1/debug/guest_example.wasm"),
        manifest_dir.join("../target/wasm32-wasi/debug/guest_example.wasm"),
        PathBuf::from("target/wasm32-wasip1/debug/guest_example.wasm"),
        PathBuf::from("target/wasm32-wasi/debug/guest_example.wasm"),
    ];

    candidates
        .into_iter()
        .find(|path| path.exists())
        .map(normalize_path)
        .ok_or_else(|| {
            anyhow!(
                "guest module not found; expected one of: {}",
                format_candidate_list(&[
                    "../target/wasm32-wasip1/debug/guest_example.wasm",
                    "../target/wasm32-wasi/debug/guest_example.wasm",
                    "target/wasm32-wasip1/debug/guest_example.wasm",
                    "target/wasm32-wasi/debug/guest_example.wasm",
                ])
            )
        })
}

fn normalize_path(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

fn format_candidate_list(paths: &[&str]) -> String {
    paths.join(", ")
}
