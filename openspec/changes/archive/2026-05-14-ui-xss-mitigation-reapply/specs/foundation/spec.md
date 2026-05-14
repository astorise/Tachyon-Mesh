# Technical Specification: Safe DOM Rendering

## 1. Migration Pattern
All dynamic views must transition from string interpolation to DOM node construction.

**Anti-Pattern (Current State):**
```typescript
// TachyonStoragePanel.ts
this.container.innerHTML = `
  <div class="kv-row">
    <span class="key">${escapeHtml(key)}</span>
    <span class="val">${escapeHtml(value)}</span>
  </div>
`;
```

**Target Pattern:**
```typescript
const row = document.createElement('div');
row.className = 'kv-row';

const keySpan = document.createElement('span');
keySpan.className = 'key';
keySpan.textContent = key; // Natively safe

const valSpan = document.createElement('span');
valSpan.className = 'val';
valSpan.textContent = value; // Natively safe

row.append(keySpan, valSpan);

// Use replaceChildren to clear old content and append the new fragment safely
this.container.replaceChildren(row);
```

## 2. Target Files for Refactoring
Based on the repository state, prioritize the following files:
* `tachyon-ui/src/components/domains/TachyonStoragePanel.ts`
* `tachyon-ui/src/components/layout/TachyonAppShell.ts`
* `tachyon-ui/src/components/routing/TachyonRoutingDashboard.ts`
* `tachyon-ui/src/views/aiOrchestration.ts`
* `tachyon-ui/src/views/routing.ts`

## 3. Utility Cleanup
Once the refactoring is complete across the `src/` directory, search for the `escapeHtml` function definition (likely in a `utils` file or embedded in the component headers) and delete it.