# Technical Specification: UI DOM Safety

## 1. Eradicating `innerHTML`
Audit and refactor `tachyon-ui/src/components/`.

**Example Pattern (`TachyonStoragePanel.ts`):**
*Before:*
```typescript
this.container.innerHTML = `
  <div class="kv-row">
    <span>${escapeHtml(key)}</span>
  </div>
`;
```

*After:*
```typescript
const row = document.createElement('div');
row.className = 'kv-row';
const keySpan = document.createElement('span');
keySpan.textContent = key; // Natively safe
row.appendChild(keySpan);

// Use replaceChildren to safely replace inner content
this.container.replaceChildren(row);
```

Apply this pattern specifically to:
- `TachyonAppShell.ts` (line ~158)
- `TachyonStoragePanel.ts` (lines ~125-160)
- `TachyonAppShellModalRoot.ts` (line ~47)