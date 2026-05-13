# Design: e2e-testing-playwright-mcp

## Overview

Adds an end-to-end testing layer covering both the Tauri UI (via Playwright) and the MCP stdio server (via a Rust integration test that spawns the binary and drives it over stdin/stdout). A dedicated GitHub Actions workflow runs the MCP tests on every push and provides an opt-in gate for the Playwright tests.

## Task 1 — Playwright Setup

`tachyon-ui/playwright.config.ts` configures Playwright against the Vite dev server (`http://localhost:1420`). Workers are capped to 1 to avoid race conditions in the single-window Tauri WebView. The `webServer` block reuses an existing server on non-CI runs and starts a fresh one in CI.

`@playwright/test ^1.53.0` is added to `devDependencies` in `package.json`.

## Task 2 — UI Happy Path Tests (`e2e/auth-to-apply.spec.ts`)

Four scenarios are tested, all adapted to the real Web Component shadow DOM structure from the `ui-component-decomposition` change:

1. **Credentials form renders** — asserts that `tachyon-iam > auth-step-credentials` is attached and that the shadow-DOM inputs `#cred-url`, `#cred-username`, `#cred-password` are visible. Playwright's CSS engine pierces shadow roots automatically via chained `.locator()` calls.

2. **Login submits and shows MFA step** — fills the credential inputs and dispatches a form submit. In offline CI (no cluster) a soft assertion tolerates the network failure; in connected mode the MFA step is expected to appear.

3. **Shell nav renders after `iam:authenticated` event** — dispatches the event via `page.evaluate()` to bypass real network auth, then asserts the app shell is visible and contains `tachyon-app-shell-nav`.

4. **Seal button visibility** — asserts `#btn-seal-apply` is hidden by default, then dispatches `config:staged` and verifies the button becomes visible.

## Task 3 — MCP stdio E2E Runner (`tachyon-mcp/tests/mcp_e2e_runner.rs`)

Three test cases:

1. **`test_initialize_returns_protocol_version`** — spawns the release (or debug) binary, sends the JSON-RPC `initialize` request, and asserts the response contains `protocolVersion: "2025-03-26"` and a `serverInfo` object. No cluster required.

2. **`test_tools_list_is_valid_jsonrpc`** — sends `tools/list` after `initialize`. Accepts either a successful tool list OR a `-32001` cluster-unreachable error; both are structurally valid JSON-RPC responses. When the cluster IS reachable, it asserts each tool entry has `name`, `description`, and `inputSchema` fields.

3. **`test_tools_list_against_live_cluster`** — guarded by `E2E_CLUSTER_URL` / `E2E_CLUSTER_PAT` env vars; skipped otherwise. Asserts the three critical tools (`tachyon_hardware_status`, `tachyon_topology_snapshot`, `tachyon_dryrun_manifest`) are present and that `tachyon_hardware_status` does not return a `-32603` internal error.

A `send_and_recv` helper wraps the stdin write + stdout readline pattern to keep tests concise.

## Task 4 — CI Workflow (`.github/workflows/e2e.yml`)

Two jobs:

- **`mcp-e2e`** — runs on every push/PR. Builds `tachyon-mcp --release`, runs the offline MCP tests (no cluster). If `E2E_CLUSTER_URL` and `E2E_CLUSTER_PAT` secrets are configured, also runs the live-cluster test.

- **`playwright-ui`** — gated behind `vars.ENABLE_PLAYWRIGHT_CI == 'true'` to avoid mandating a headless browser environment on every CI run. When enabled it installs Playwright with Chromium, runs `npx playwright test`, and uploads the HTML report as a build artifact.

## Pre-existing Bug Fixes

While implementing this change, three pre-existing errors in `tachyon-client/src/lib.rs` were repaired:
1. `current_connection_config()` was called but doesn't exist — replaced with `require_connection()`.
2. `RuntimeMetrics` initializer was missing `vram_utilization_pct` and `ram_offload_active` fields.
3. `post_admin_json::<serde_json::Value>` was called with one type parameter; it requires two (`I` and `T`).
