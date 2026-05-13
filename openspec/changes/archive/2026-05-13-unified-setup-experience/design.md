# Design: unified-setup-experience

## Overview

Replaces the fragmented multi-step manual README with a single idempotent bootstrap script that takes a fresh clone to a working environment in under two minutes.

## Task 1 & 2 — `scripts/setup.sh` (Linux / macOS)

Seven sequential phases, all guarded by `set -euo pipefail`:

1. **Prerequisites** — `command -v cargo`, `rustup`, `npm`. Each missing tool prints a coloured `✘` message and exits 1 with the install URL.
2. **WASM targets** — `rustup target add wasm32-wasip1 wasm32-wasip2` (idempotent).
3. **Core binaries** — `cargo build --release --bin core-host --bin tachyon-mcp`.
4. **Guest artifacts** — `bash scripts/build-guest-artifacts.sh` (skippable with `--skip-guests`).
5. **UI dependencies** — `cd tachyon-ui && npm install` (skippable with `--skip-ui`).
6. **Cross-layer validation** — `bash scripts/validate_cross_layer.sh` to assert route/client parity.
7. **Success banner** — colour-coded block with exact startup commands and a ready-to-paste MCP JSON snippet using the absolute path to the built `tachyon-mcp` binary.

The script uses `tput` for colour with a graceful fallback when no terminal is present (CI).

## Task 3 — README.md Quick Start

The "Quick Start" section is rewritten to lead with the one-command bootstrap:

```bash
./scripts/setup.sh          # Linux / macOS
.\scripts\setup.ps1         # Windows PowerShell
```

Manual step-by-step instructions are replaced by an "After Setup" block showing only the two terminal commands needed once the script has run.

## Task 4 — `scripts/setup.ps1` (Windows / PowerShell)

A PowerShell equivalent covering the same seven phases:
- Uses `Get-Command` for prerequisite checks with `Write-Host` colour output.
- Falls back gracefully when `bash` is absent (Git Bash / WSL not installed): guest build and cross-layer validation are skipped with an informational yellow message.
- Resolves `$PSScriptRoot` for an absolute path to `tachyon-mcp.exe` in the MCP banner.
- Accepts `-SkipGuests` and `-SkipUI` switches matching the bash flags.
