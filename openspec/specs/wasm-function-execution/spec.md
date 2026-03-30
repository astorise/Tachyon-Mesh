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
The `guest-example` module SHALL compile as a WebAssembly component targeting `wasm32-wasip2` and export the `tachyon:mesh/handler.handle-request` function, returning a typed HTTP-like response without relying on WASI stdin/stdout for the request boundary.

#### Scenario: `guest-example` returns a typed response for non-empty payloads
- **WHEN** a caller invokes `handle-request` with a `request` record whose `body` contains UTF-8 payload bytes
- **THEN** the component returns a `response` record with status `200`
- **AND** the response body contains `FaaS received: <payload>`

#### Scenario: `guest-example` handles empty payloads
- **WHEN** a caller invokes `handle-request` with an empty `body`
- **THEN** the component returns a `response` record with status `200`
- **AND** the response body contains `FaaS received an empty payload`

### Requirement: Host supports command-style WASI guest entrypoints
The `core-host` runtime SHALL execute guest modules that export either the existing `faas_entry` function or the standard WASI command entrypoint `_start`, while preserving the same stdin/stdout contract for both.

#### Scenario: Guest module exposes `_start` instead of `faas_entry`
- **WHEN** the host loads a guest module that does not export `faas_entry` but does export `_start`
- **THEN** the host invokes `_start`
- **AND** the guest still receives the HTTP request body through WASI stdin
- **AND** the host still returns the captured WASI stdout as the HTTP response body

### Requirement: Workspace defines a shared WIT contract for typed guest invocation
The workspace SHALL define a `wit/tachyon.wit` package that describes the typed request and response contract used by Component Model guests.

#### Scenario: The shared WIT file declares the guest handler world
- **WHEN** a developer inspects `wit/tachyon.wit`
- **THEN** the file defines the `tachyon:mesh` package
- **AND** it defines a `handler` interface with `request` and `response` records
- **AND** it defines a `faas-guest` world that exports the `handler` interface

### Requirement: Host prefers typed component guests and preserves legacy WASI fallback
The `core-host` runtime SHALL resolve guest artifacts from the workspace or packaged guest directory, instantiate a WebAssembly component when the artifact implements the `faas-guest` world, and fall back to the existing WASI preview1 execution path when the artifact is a legacy module.

#### Scenario: Host executes `guest-example` through the Component Model
- **WHEN** `core-host` receives an HTTP request for `/api/guest-example`
- **AND** the resolved `guest_example.wasm` artifact is a valid WebAssembly component exporting `tachyon:mesh/handler`
- **THEN** the host instantiates it with `wasmtime::component::Component`
- **AND** passes the HTTP method, URI, and request body through the typed WIT `request` record
- **AND** returns the typed WIT `response` status and body to the client

#### Scenario: Host falls back to the legacy WASI pipeline for non-component guests
- **WHEN** `core-host` resolves a guest artifact that does not decode as a WebAssembly component
- **THEN** it instantiates the artifact with the existing WASI preview1 module pipeline
- **AND** the guest still receives the HTTP request body through WASI stdin
- **AND** the host still returns the captured stdout as the HTTP response body

### Requirement: Component guests retrieve secrets through a typed host vault import
The workspace SHALL define a `secrets-vault` WIT interface in `wit/tachyon.wit`, and `core-host` SHALL implement that import for `faas-guest` components without exposing the same secrets through the guest environment block.

#### Scenario: Vault is disabled at compile time
- **WHEN** `core-host` runs without `--features secrets-vault`
- **THEN** a `faas-guest` component can still call `get-secret("DB_PASS")`
- **AND** the host returns `vault-disabled`
- **AND** `std::env::var("DB_PASS")` inside the guest remains unset

#### Scenario: Authorized guest receives a sealed secret
- **WHEN** `core-host` is built with `--features secrets-vault`
- **AND** `/api/guest-example` is sealed with `allowed_secrets: ["DB_PASS"]`
- **THEN** the guest receives `super_secret_123` from `get-secret("DB_PASS")`
- **AND** the guest still cannot read `DB_PASS` from its environment block

#### Scenario: Unauthorized guest is denied
- **WHEN** a component guest requests a secret that is not granted by its sealed route metadata
- **THEN** the host returns `permission-denied`
- **AND** the secret value is not disclosed

### Requirement: Host mounts sealed route volumes into the guest filesystem
The `core-host` runtime SHALL preopen every sealed route volume into the request-scoped WASI
context for both legacy WASI guests and Component Model guests, while honoring the sealed
read-only flag.

#### Scenario: Stateful guest persists data through a mounted directory
- **WHEN** `/api/guest-volume` is sealed with a host directory mounted at `/app/data`
- **AND** a client sends `POST Hello Stateful World` to `/api/guest-volume`
- **AND** the client later sends `GET /api/guest-volume`
- **THEN** the guest writes `state.txt` under `/app/data`
- **AND** the subsequent `GET` returns `Hello Stateful World`
- **AND** the host filesystem contains the persisted file in the mounted host directory

#### Scenario: Read-only guest volume denies writes
- **WHEN** a sealed route volume is mounted with `readonly = true`
- **AND** the guest attempts to write under the configured `guest_path`
- **THEN** the guest receives a WASI permission error
- **AND** the host volume contents are not modified
