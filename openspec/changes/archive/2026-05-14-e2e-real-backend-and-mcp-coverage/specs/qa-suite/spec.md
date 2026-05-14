# Technical Specification: Advanced QA Suite

## 1. Playwright Network Routing (`tachyon-ui/e2e/auth-to-apply.spec.ts`)
Instead of dispatching custom events, intercept the network calls that Tauri's Rust backend makes to the `core-host`.

```typescript
import { test, expect } from '@playwright/test';

test.beforeEach(async ({ page }) => {
  // Mock the login endpoint
  await page.route('**/admin/auth/login', async route => {
    await route.fulfill({ json: { token: 'mock-jwt-token', requires_mfa: true } });
  });

  // Mock the MFA verification endpoint
  await page.route('**/admin/auth/mfa/verify', async route => {
    await route.fulfill({ json: { success: true } });
  });

  // Mock the seal_and_apply endpoint with an artificial delay
  await page.route('**/admin/manifest/apply', async route => {
    // Simulate 2 seconds of cryptographic/network delay
    await new Promise(resolve => setTimeout(resolve, 2000));
    await route.fulfill({ json: { status: 'applied', version: 'v2' } });
  });
});

test('Critical Path: Login, view topology, and seal manifest', async ({ page }) => {
  await page.goto('/');
  
  // Real DOM interactions that trigger the real fetch() calls
  await page.fill('#pat-token', 'mock-test-token');
  await page.click('button:has-text("Connect")');
  
  // Verify MFA state triggered by the network mock
  await expect(page.locator('auth-step-mfa')).toBeVisible();
  // ... continue user journey
});
```

## 2. Advanced MCP Runner (`tachyon-mcp/tests/mcp_e2e_runner.rs`)
Add test cases that exercise the mutator logic and the newly implemented rate-limit error codes.

```rust
// Inside test_mcp_all_tools_sanity()

// Test 1: KV Put (Happy Path)
let kv_req = r#"{"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "tachyon_kv_put", "arguments": {"namespace": "test", "key": "foo", "value": "bar"}}}"#;
stdin.write_all(kv_req.as_bytes()).unwrap();
// Read stdout, assert result contains "success"

// Test 2: Rate Limit Trigger (assuming limit is 2/min for canary_split)
let canary_req = r#"{"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "tachyon_canary_split", "arguments": {"function": "demo", "weight_pct": 50}}}"#;

// Send 3 times rapidly
for _ in 0..3 {
    stdin.write_all(canary_req.as_bytes()).unwrap();
    stdin.write_all(b"\n").unwrap();
}

// The 3rd response should be an error
// Parse the 3rd JSON-RPC response
// assert_eq!(response["error"]["code"], -32002);
// assert!(response["error"]["data"]["retry_after_ms"].is_number());
```