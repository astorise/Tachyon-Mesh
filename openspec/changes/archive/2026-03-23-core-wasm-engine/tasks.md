## 1. Workspace Setup

- [x] 1.1 Create a root `Cargo.toml` that declares a workspace with `core-host` and `guest-example`.
- [x] 1.2 Create the `core-host` binary crate and add `tokio`, `wasmtime`, `wasmtime-wasi`, and `anyhow`.
- [x] 1.3 Create the `guest-example` library crate and configure it as a `cdylib` for `wasm32-wasip1`.

## 2. Guest Module

- [x] 2.1 Replace the default code in `guest-example/src/lib.rs`.
- [x] 2.2 Export a public `faas_entry` function that writes `Hello from WASM FaaS!` to standard output.

## 3. Host Runtime

- [x] 3.1 Add a `#[tokio::main]` entrypoint in `core-host/src/main.rs`.
- [x] 3.2 Initialize `wasmtime::Engine`, `wasmtime::Linker`, and add WASI bindings through `wasmtime_wasi::add_to_linker`.
- [x] 3.3 Build a `WasiCtx` with inherited stdio and store it in a `wasmtime::Store`.
- [x] 3.4 Load the compiled guest module from the workspace `target` directory, instantiate it, resolve `faas_entry`, and invoke it.
- [x] 3.5 Surface host failures through `anyhow::Result`.

## 4. Verification

- [x] 4.1 Add the `wasm32-wasip1` target with `rustup target add wasm32-wasip1`.
- [x] 4.2 Build `guest-example` for the `wasm32-wasip1` target.
- [x] 4.3 Build and run `core-host`, then verify the guest output is printed through inherited stdio.
