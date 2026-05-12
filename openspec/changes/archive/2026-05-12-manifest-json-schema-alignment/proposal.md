# Proposal: Manifest JSON Schema Alignment & WASM Validation

## Context
The usability audit identified a P0 reliability issue for LLM agents: the `tachyon_dryrun_manifest` and `tachyon_apply_manifest` MCP tools accept a `manifest` argument defined blindly as `{"type": "object"}`. Furthermore, `dryrun_manifest` returns flat text errors, making self-correction impossible for agents.

## Problem
1. **Agent Blindness:** Without a JSON Schema describing the expected manifest shape, an LLM agent must "guess" the syntax, leading to high failure rates.
2. **Lack of Structured Self-Correction:** Validation errors are plain text strings. An agent cannot programmatically map the error to the specific JSON path to fix it.
3. **Monolithic Validation Risk:** Implementing heavy JSON validation directly in the `core-host` violates the project's design philosophy, which favors offloading business logic to Rust FaaS (WASM) modules.

## Proposed Solution
1. **Single Source of Truth (`core-host`):** Use the `schemars` crate in `core-host` to automatically derive a JSON Schema from the internal Rust `Manifest` structs and serve it via `GET /admin/schema/manifest`.
2. **WASM-Delegated Validation (`system-faas-config-api`):** Route all `dryrun` and validation requests to the system FaaS. This WASM guest will load the schema, validate the incoming JSON manifest against it, and generate structured error reports.
3. **Structured Errors:** Return a structured array of `ValidationError { path, message, error_code }` to the client.
4. **MCP Alignment:** Fetch the schema from `core-host` at MCP startup and inject it natively into the JSON-RPC tool definitions.

## Impact
- **Agentic Usability:** Agents get IntelliSense-like schema validation natively through the MCP protocol.
- **Architecture:** Keeps `core-host` lean by dogfooding the WASM engine for system configuration tasks.