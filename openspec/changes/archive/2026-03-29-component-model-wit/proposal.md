## Why

Tachyon Mesh passe encore les requêtes HTTP au guest Rust via `stdin`/`stdout`, ce qui force un contrat implicite et peu typé. Introduire le WebAssembly Component Model pour `guest-example` permet de verrouiller une frontière mémoire explicite sans casser les invités WASI existants du dépôt.

## What Changes

- Add a shared `wit/tachyon.wit` contract describing the typed `request` and `response` records for a `faas-guest` world.
- Refactor `guest-example` to compile as a `wasm32-wasip2` WebAssembly component using `wit-bindgen`.
- Update `core-host` to prefer executing typed component guests and keep the current WASI preview1 path as a fallback for legacy guests.
- Update build and packaging flows so CI, Docker, and local instructions produce the component artifact for `guest-example`.

## Capabilities

### New Capabilities

- None.

### Modified Capabilities

- `wasm-function-execution`: migrate `guest-example` to a typed WIT component contract while preserving legacy WASI guest execution as a fallback.

## Impact

- Affects `core-host`, `guest-example`, `Dockerfile`, `.github/workflows/ci.yml`, and `README.md`.
- Adds a shared `wit/` directory and a new `wit-bindgen` dependency.
- Changes the primary build target for `guest-example` from `wasm32-wasip1` to `wasm32-wasip2`.
