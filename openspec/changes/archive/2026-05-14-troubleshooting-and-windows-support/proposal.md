# Proposal: Comprehensive Troubleshooting & Windows Support

## Context
The post-Codex usability audit identified critical gaps in the developer and operator onboarding experience. Specifically, there is no centralized troubleshooting guide for common setup failures, Windows binaries are missing from the automated releases, and the MCP server silently fails if it cannot fetch the dynamic manifest schema from the core host.

## Problem
1. **Friction on Errors:** Users encountering missing `wasm32-wasip2` targets, occupied ports (8080), missing Tauri dependencies (WebKitGTK, MSVC), or `integrity.lock` signature failures have no reference to solve their issues.
2. **Windows Exclusion:** The `setup.ps1` script falls back to a 20-minute local build because `release.yml` does not emit pre-compiled `x86_64-pc-windows-msvc` artifacts.
3. **Agent Blindness:** If `tachyon-mcp` fails to fetch the manifest schema on startup, it falls back to a generic schema but does not warn the LLM agent that it is operating in a degraded mode.

## Proposed Solution
1. **Centralized FAQ:** Create a rich `TROUBLESHOOTING.md` in the root directory covering the 15 most common failure modes across FaaS, Tauri, AI (ONNX/VRAM), and K8s. Link it prominently in the `README.md`.
2. **Windows CI Target:** Expand `.github/workflows/release.yml` to cross-compile and attach Windows binaries (`.zip`) alongside Linux and macOS tarballs.
3. **MCP Warning Propagation:** Add `tracing::warn!` when the MCP schema fetch fails, and inject a `warnings` array in the JSON-RPC response for `tools/list` so the agent is explicitly aware of the fallback state.

## Impact
- **Developer Experience:** Radically reduces time-to-resolution for environment-specific bugs.
- **Reach:** First-class Windows support unlocks the largest desktop OS demographic.
- **Agentic Reliability:** LLMs can adapt their reasoning if they know the strict JSON schema is temporarily unavailable.