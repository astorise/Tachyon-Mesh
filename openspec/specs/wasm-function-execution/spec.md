# wasm-function-execution Specification

## Purpose
TBD - created by archiving change core-wasm-engine. Update Purpose after archive.
## Requirements
### Requirement: Rust workspace defines the host and guest crates
The project SHALL define a Cargo workspace that includes a `core-host` binary crate and a `guest-example` guest crate configured as a `cdylib` for `wasm32-wasip1` builds.

#### Scenario: Workspace members are declared
- **WHEN** a developer inspects the root workspace configuration
- **THEN** the workspace lists `core-host` and `guest-example` as members
- **AND** the guest crate is configured to build as a `cdylib`

### Requirement: Guest module exposes a stable FaaS entrypoint
The `guest-example` module SHALL export a function named `faas_entry` that can be invoked by the host and writes `Hello from WASM FaaS!` to standard output when executed under WASI.

#### Scenario: Guest entrypoint emits the expected greeting
- **WHEN** the host invokes `faas_entry` from the compiled guest module
- **THEN** the guest writes `Hello from WASM FaaS!` to standard output

### Requirement: Host executes the guest module with WASI stdio inheritance
The `core-host` runtime SHALL load the compiled guest module from the workspace `target` directory, preferring `target/wasm32-wasip1/debug/guest_example.wasm` and tolerating the legacy `wasm32-wasi` path, then instantiate it with Wasmtime and WASI support and invoke the guest entrypoint while inheriting the host standard output and error streams.

#### Scenario: Host runs a compiled guest module successfully
- **WHEN** `core-host` starts and the compiled guest module exists at the expected workspace target path
- **THEN** the host instantiates the module with a WASI context that inherits stdio
- **AND** the host invokes the exported `faas_entry` function
- **AND** the process exits successfully after guest execution

