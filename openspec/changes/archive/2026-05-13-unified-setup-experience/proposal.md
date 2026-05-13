# Proposal: Unified First-Run Setup Experience

## Context
The P2 audit highlighted that while Tachyon-Mesh is powerful, the initial onboarding is highly fragmented. Currently, the `README.md` lists manual steps, but relies on assumed knowledge regarding WASM targets, UI dependencies, and MCP configuration. 

## Problem
A new contributor spends 10 to 20 minutes running disparate commands across different directories (`core-host`, `tachyon-ui`, `scripts/build-guest-artifacts.sh`) before seeing the system work. This friction discourages adoption and makes automated CI/Agent bootstrapping unnecessarily complex.

## Proposed Solution
Introduce a comprehensive `scripts/setup.sh` (or `Makefile` default target) that acts as a deterministic bootstrap script. It will:
1. Verify system prerequisites (Rust, Node, npm).
2. Automatically add the `wasm32-wasip2` target.
3. Build all guest artifacts and generate an initial `integrity.lock`.
4. Install UI dependencies.
5. Print a clear, formatted "Success" banner containing the exact JSON snippet needed for Claude Desktop MCP integration and the commands to start the UI and Host.

## Impact
- **Time to First Request:** Reduced from 15 minutes to < 2 minutes.
- **Agentic Autonomy:** Agents cloning the repo can just run `./scripts/setup.sh` and have a guaranteed working environment.