# Proposal: Zero-Build Operator Deployment Experience

## Context
While we have a robust `setup.sh` script for contributors to compile the project from source, we currently force end-users and operators into the same path. Tachyon-Mesh already has a CI/CD pipeline (`release.yml`) and Kubernetes manifests (`manifests/deploy.yaml`), but they are not leveraged effectively for onboarding.

## Problem
End-users wanting to simply deploy a WASM function or route AI traffic on Tachyon-Mesh currently have to install Rust, the `wasm32-wasip2` target, and Node.js, and then wait for a full compilation. This creates massive friction for infrastructure teams and standalone LLM agents who just need the binaries.

## Proposed Solution
1. **The Download Script (`get-tachyon.sh`):** Create a zero-build installation script that fetches the latest compiled release binaries (`core-host` and `tachyon-mcp`) from GitHub Releases for the user's OS/Arch.
2. **Kubernetes First:** Promote the `manifests/deploy.yaml` in the documentation as the standard way to run Tachyon in production or homelabs.
3. **README Refactoring:** Split the "Quick Start" section of the `README.md` into two clear paths: "For Operators (Zero-Build)" and "For Contributors (Build from Source)".

## Impact
- **Adoption:** Drops the time-to-evaluate for an operator from ~15 minutes to ~10 seconds.
- **Agentic Efficiency:** LLM agents acting as infrastructure operators can bypass source-code manipulation entirely by downloading the release artifacts.