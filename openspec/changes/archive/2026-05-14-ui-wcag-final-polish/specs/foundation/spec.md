# Technical Specification: A11y Finishes

## 1. Escape Key Handler (`tachyon-ui/src/utils/a11y.ts`)
Update the `trapFocus` signature to accept an optional teardown/close callback.

```typescript
export function trapFocus(element: HTMLElement, onClose?: () => void) {
  // ... existing focusable elements logic ...

  const keydownHandler = function(e: KeyboardEvent) {
    // Escape Key logic
    if (e.key === 'Escape' || e.key === 'Esc') {
      if (onClose) {
        onClose();
        e.preventDefault();
        return;
      }
    }

    // ... existing Tab/Shift+Tab logic ...
  };

  element.addEventListener('keydown', keydownHandler);
  if (focusableEls[0]) (focusableEls[0] as HTMLElement).focus();

  // Return a cleanup function
  return () => {
    element.removeEventListener('keydown', keydownHandler);
  };
}
```

*Note: Update `TachyonIAM.ts` and `TachyonAppShellModalRoot.ts` to pass their respective close/dismiss functions to `trapFocus`.*

## 2. Global Loader Announcement (`TachyonAppShell.ts`)
Update the global loader injection (around line 334) to include `aria-live`.

```typescript
// Add aria-live to the loader container
loader.setAttribute('aria-live', 'polite');
loader.innerHTML = `
  <div class="sr-only">Applying configuration...</div>
  <div class="spinner border-t-blue-500 border-4 rounded-full w-12 h-12 animate-spin" aria-hidden="true"></div>
`;
```

## 3. Users Panel Modal (`TachyonUsersPanel.ts`)
Locate the user audit modal and apply the standard attributes.

```html
<div 
  class="modal-backdrop fixed inset-0 z-[80] bg-black/50" 
  role="dialog" 
  aria-modal="true" 
  aria-labelledby="audit-modal-title"
>
  <h2 id="audit-modal-title" class="sr-only">User Audit Details</h2>
  </div>
```
*Ensure `trapFocus` is invoked immediately after this DOM node is rendered, passing a callback to close the panel.*