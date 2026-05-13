# Technical Specification: Playwright for Tauri UI

## 1. Setup (`tachyon-ui/package.json`)
Add Playwright dependencies specifically configured for testing Tauri applications (which require a custom build hook to spawn the Rust binary).

```bash
npm install -D @playwright/test
```

## 2. Test Configuration (`tachyon-ui/playwright.config.ts`)
Configure Playwright to launch the Tauri dev server before running tests.

```typescript
import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  webServer: {
    command: 'npm run tauri dev',
    port: 1420,
    timeout: 120 * 1000,
    reuseExistingServer: !process.env.CI,
  },
  use: {
    baseURL: 'http://localhost:1420',
  },
});
```

## 3. Core E2E Flow (`tachyon-ui/e2e/auth-to-apply.spec.ts`)
Write the "Happy Path" test.

```typescript
import { test, expect } from '@playwright/test';

test('Critical Path: Login, view topology, and seal manifest', async ({ page }) => {
  await page.goto('/');

  // 1. Auth Step
  await page.fill('#pat-token', 'mock-test-token');
  await page.click('button:has-text("Connect")');

  // 2. MFA Step
  await expect(page.locator('auth-step-mfa')).toBeVisible();
  await page.fill('#mfa-code', '123456'); // Assuming mock env bypasses real TOTP
  await page.click('button:has-text("Verify")');

  // 3. Topology
  await expect(page.locator('app-shell-nav')).toBeVisible();
  await page.click('nav a[href="#topology"]');
  await expect(page.locator('.topology-canvas')).toBeVisible();
});
```