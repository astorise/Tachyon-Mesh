# Implementation Tasks

- [x] **Task 1: Playwright Mocks** — Refactor auth-to-apply.spec.ts with page.addInitScript Tauri mock and page.route() for HTTP endpoints.
- [x] **Task 2: Playwright Load State Testing** — New test asserts aria-busy=true and loader visible while apply mock delays response.
- [x] **Task 3: MCP Mutator Tests** — test_kv_put_is_valid_jsonrpc added to mcp_e2e_runner.rs.
- [x] **Task 4: MCP Error Code Validation** — test_canary_split_rate_limit_returns_32002 triggers 3 rapid calls and asserts -32002 on the 3rd.
