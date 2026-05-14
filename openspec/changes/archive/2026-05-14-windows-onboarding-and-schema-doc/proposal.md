# Proposal: Windows Zero-Build Script & Schema Integration

## Context
The T+3 usability audit highlighted a platform asymmetry and a documentation gap in our onboarding process. While we recently added Windows `.zip` artifacts to our GitHub Releases and exposed the `integrity.lock` JSON schema on the core-host, we left the developer experience incomplete.

## Problem
1. **Windows Asymmetry:** Linux/macOS users have a frictionless `get-tachyon.sh` script, but Windows users hit a wall because the bash script explicitly rejects their OS. They are forced to manually navigate GitHub, download, and extract the ZIP, missing out on the automated MCP config generation.
2. **Hidden Feature:** The `/admin/schema/integrity-lock` endpoint exists, but without documentation or IDE integration snippets, developers and external validators do not know how to leverage it to prevent syntax errors before deployment.

## Proposed Solution
1. **Symmetrical PowerShell Script:** Create `scripts/get-tachyon.ps1` that mirrors the bash script logic: fetching the latest release from the GitHub API, downloading the `.zip`, extracting it via `Expand-Archive`, and printing the Claude Desktop config block.
2. **Schema Documentation:** Create a dedicated guide in `docs/ide-integration.md` (and reference it in the README) showing developers how to bind the dynamic JSON schema to their IDEs (VS Code / JetBrains).

## Impact
- Delivers a true "Zero-Build" experience for Windows operators.
- Transforms the `integrity.lock` schema from a hidden API endpoint into an active, highly visible DX feature that prevents misconfigurations.