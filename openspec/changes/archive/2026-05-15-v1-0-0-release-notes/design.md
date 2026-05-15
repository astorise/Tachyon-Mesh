# Design: v1.0.0 Release Notes

## What Was Built

Official GA release documentation and a coordinated version bump across all workspace crates and the UI package.

### Task 1 — `CHANGELOG.md`

Created at the repository root. Five sections covering the complete audit cycle:
- **Security & Supply Chain** — keyless signing, SBOM, SHA-256 verification, XSS immunity, CI hardening
- **AI Agents & MCP** — pre-auth param validation, rate limiting, error taxonomy, E2E coverage, `tools/list` warnings
- **UI & Accessibility (WCAG AAA)** — focus restoration, screen reader announcements, `<dialog>` safety docs, apply loader, component decomposition
- **Kubernetes & Infrastructure** — hardened manifest, NetworkPolicy, dynamic OpenAPI, IDE schema integration, GPU homelab
- **Developer Experience** — setup scripts, zero-build installers, TROUBLESHOOTING.md, ide-integration.md

Drafted from the archived OpenSpec changes completed across this audit cycle.

### Task 2 — README.md Security Posture Section

Injected `## 🔒 Enterprise Security Posture` immediately before the FaaS Guests / WIT Contracts section (after Quick Start). Lists five bullet points covering verified binaries, cryptographic signatures, SBOM, hardened K8s manifest, and XSS immunity. Links to `CHANGELOG.md` for the complete feature list.

### Task 3 — Global Version Bump

- `version = "0.1.0"` → `version = "1.0.0"` in all **54 Rust crates** (workspace members in `core-host`, `crates/`, `examples/`, `systems/`, `faas-sdk`, `tachyon-client`, `tachyon-mcp`, `tachyon-ui`, `turboquant-sys`).
- `"version": "0.1.0"` → `"version": "1.0.0"` in `tachyon-ui/package.json`.
- No `tauri.conf.json` present in the repo (Tauri source is managed separately).
- UI build verified clean post-bump: `npm run build` → 74 modules, 0 errors.

## Files Changed
- `CHANGELOG.md` (new)
- `README.md`
- `tachyon-ui/package.json`
- 54 × `*/Cargo.toml`
