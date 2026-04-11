# Proposal: Change 064 - UI Exorcism & Triad Finalization

## Why
The desktop wrapper still contains the legacy manifest-generation code path inherited from the old `tachyon-cli`. That startup-time argument scan makes the GUI crate behave like a terminal application, which is the wrong boundary after the client triad split.

## What Changes
1. Remove startup-time CLI parsing and manifest-generation logic from `tachyon-ui`.
2. Reduce `tachyon-ui` to a pure Tauri desktop entrypoint that immediately boots the window and exposes `get_engine_status`.
3. Keep `tachyon-mcp` strict on `stdio`, with JSON-RPC payloads on `stdout` and diagnostics on `stderr` only.
4. Stop invoking the GUI binary as a manifest generator in build infrastructure.
