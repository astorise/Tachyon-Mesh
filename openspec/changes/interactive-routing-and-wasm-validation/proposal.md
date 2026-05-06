# Proposal: Interactive Routing Forms & Wasmtime Validation

## Context
We successfully established the Tauri IPC bridge between the Vanilla JS frontend and the Rust backend. However, the frontend currently "scrapes" static HTML to build its JSON payload, and the Rust backend only performs a superficial string match on the `api_version` instead of executing the true WIT contract.

## Problem
1. **Frontend**: Users cannot dynamically type new routes or gateways; the UI is read-only.
2. **Backend**: The Rust control plane is bypassing the Zero-Panic Wasm architecture, accepting potentially malformed JSON if the `api_version` happens to match.

## Solution
Complete the Vertical Slice for Domain 1 (Traffic Management):
1. **Interactive UI**: Refactor `tachyon-ui/src/views/routing.ts` to include actual `<input>` and `<select>` fields for L4 Gateways and L7 Routes. Update the controller to read `.value` instead of `textContent`.
2. **Wasm Validation Engine**: Update the Rust Tauri command (`apply_configuration`) to instantiate the Wasmtime engine, load a mocked `system-faas-config-api.wasm` (or perform strict serde validation mimicking the WIT schema if the `.wasm` binary isn't compiled yet), ensuring the payload conforms perfectly to `config-routing.wit`.