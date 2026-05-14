# Design: windows-onboarding-and-schema-doc

## Task 1 — `scripts/get-tachyon.ps1`

PowerShell one-liner installer for Windows operators. Mirrors `get-tachyon.sh` in structure:
- Accepts `-Version <tag>` (defaults to latest via GitHub Releases API) and `-Dir <path>` (defaults to `./`).
- Constructs the artifact URL matching the release workflow output: `tachyon-mesh-{VERSION_NO_V}-windows-x86_64.zip`.
- Downloads with `Invoke-WebRequest`, extracts with `Expand-Archive`, removes the zip.
- Prints a success banner with `core-host.exe` path and a ready-to-paste MCP JSON snippet using the absolute binary path.
- Fails with `$ErrorActionPreference = "Stop"` and a helpful message if the download returns non-200.

## Task 2 — `docs/ide-integration.md`

Comprehensive guide covering four environments:
- **VS Code**: `json.schemas` settings.json binding for `integrity.lock` and manifests; YAML modeline for Red Hat YAML extension; OpenAPI viewer for the full API.
- **JetBrains**: JSON Schema Mappings GUI walkthrough + built-in HTTP Client `.http` snippet.
- **Neovim/LSP**: `jsonls` setup example using nvim-lspconfig.
- **Offline/Air-Gapped**: `curl` commands to snapshot schemas into `.schemas/` and point IDE to local paths.

Schema endpoint table documents all three endpoints: `manifest`, `integrity-lock`, `openapi.json`.

## Task 3 — README Updates

- Path A "Operators" section gains a **Windows (PowerShell)** block with the `irm ... | iex` one-liner and optional `-Version`/`-Dir` flags.
- New "🛠️ IDE Integration & Schema Validation" section (before GPU/Homelab) shows the VS Code quick setup snippet and a YAML modeline, then links to `docs/ide-integration.md`.
