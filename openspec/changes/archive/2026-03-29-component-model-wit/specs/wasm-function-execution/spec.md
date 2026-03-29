## ADDED Requirements

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

## MODIFIED Requirements

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

## REMOVED Requirements

### Requirement: Host executes the guest module with WASI stdio inheritance
**Reason**: The primary execution contract for `guest-example` now uses the WebAssembly Component Model instead of raw WASI stdio.
**Migration**: Use the new typed component execution path for component guests; legacy WASI guests continue to run through the fallback module pipeline.
