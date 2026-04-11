# Proposal: Change 062 - Unified Client Triad

## Why
The repository currently couples control-plane inspection logic to the Tauri desktop crate. Adding an MCP server on top of that would duplicate `integrity.lock` parsing, workspace discovery, and runtime status reporting. We need a shared Rust client layer so the human UI and the AI-facing server both consume the same code.

## What Changes
1. Add a new workspace library crate named `tachyon-client` for shared local status and lockfile access.
2. Rename the existing `tachyon-cli` project to `tachyon-ui` and keep the desktop Tauri wrapper focused on presentation plus manifest generation.
3. Add a new `tachyon-mcp` binary crate that serves a minimal JSON-RPC loop over `stdin` / `stdout` while delegating status queries to `tachyon-client`.
4. Update workspace members, CI/CD paths, Docker build steps, and OpenSpec references so the renamed project still builds cleanly.
