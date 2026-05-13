# Implementation Tasks

- [x] **Task 1: Playwright Installation**
  - Initialize Playwright in `tachyon-ui/`.
  - Configure `playwright.config.ts` to hook into the Tauri dev process.

- [x] **Task 2: UI Happy Path Test**
  - Create `tachyon-ui/e2e/auth-to-apply.spec.ts`.
  - Implement the DOM interactions matching the decomposed `<auth-step-credentials>` and `<auth-step-mfa>` components.

- [x] **Task 3: MCP E2E Harness**
  - Create `tachyon-mcp/tests/mcp_e2e_runner.rs`.
  - Write a suite that simulates an LLM client communicating over `stdio`.
  - Assert that `tools/list` returns the correct schema (including the dynamic manifest schema) and that read-only tools execute without JSON-RPC errors.

- [x] **Task 4: CI Automation**
  - Update `.github/workflows/ci.yml` (or create a specific `e2e.yml`).
  - Add steps to build the `core-host`, start it in the background, run the MCP tests, and run the Playwright tests.
