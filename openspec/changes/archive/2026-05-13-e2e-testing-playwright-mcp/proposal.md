# Proposal: E2E Testing for UI and MCP

## Context
The P2 audit recommendation highlighted a lack of End-to-End (E2E) testing. While we have limited component tests, we lack confidence that the complete critical path works as expected in a real environment. This is especially true for the complex Tauri IAM flow and the JSON-RPC contract of the MCP server.

## Problem
1. **UI Fragility:** A single CSS or component change in the monolith could break the MFA sealing workflow without CI failing.
2. **Agent Reliability:** If the `tachyon-mcp` tool contract drifts from the `core-host` capabilities, LLM agents silently fail. We have no automated way to verify that all 14 (soon 22+) tools respond correctly to an agent's requests.

## Proposed Solution
1. **Tauri Playwright Integration:** Introduce `@playwright/test` to the `tachyon-ui` project. Write a core user journey test covering: `Login -> MFA -> Topology rendering -> Manifest Seal -> Apply`.
2. **MCP Inspector CLI:** Use the official `@modelcontextprotocol/inspector` (or a lightweight custom Rust client script) in the CI pipeline to dynamically spawn `tachyon-mcp`, ping all registered tools with mock parameters, and assert standard `200 OK` JSON-RPC responses.

## Impact
- **Confidence:** Merging to `main` guarantees that human operators can log in and AI agents can execute tools.
- **Maintainability:** Acts as living documentation for the exact payloads required by both interfaces.