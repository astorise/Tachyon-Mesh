# Technical Specification: A11y Finishes

## 1. Focus Restoration & Documentation (`tachyon-ui/src/utils/a11y.ts`)
Update the `trapFocus` utility to handle focus restoration automatically and add defensive documentation.

```typescript
/**
 * Traps keyboard focus within a specified DOM element.
 * * WARNING: Do NOT use this utility on native HTML5 `<dialog>` elements, 
 * as modern browsers (Chromium/WebKit) handle focus trapping and Escape-to-close 
 * natively. This is exclusively for custom `role="dialog"` overlays.
 *
 * @param element The container element to trap focus inside.
 * @param onClose Optional callback triggered when the Escape key is pressed.
 * @returns A cleanup function to remove event listeners and restore focus.
 */
export function trapFocus(element: HTMLElement, onClose?: () => void) {
  // 1. Capture the element that had focus before the modal opened
  const previousFocus = document.activeElement as HTMLElement | null;

  const focusableEls = element.querySelectorAll<HTMLElement>(
    'a[href], button, textarea, input[type="text"], input[type="radio"], input[type="checkbox"], select, [tabindex]:not([tabindex="-1"])'
  );
  
  // ... existing Keydown, Tab, and Escape logic ...

  // 2. Return an enhanced cleanup function
  return () => {
    element.removeEventListener('keydown', keydownHandler);
    // Restore focus to the original trigger element
    if (previousFocus && typeof previousFocus.focus === 'function') {
      previousFocus.focus();
    }
  };
}
```

## 2. Async Completion Announcement (`tachyon-ui/src/components/layout/TachyonAppShell.ts`)
Ensure the `finally` or success/catch blocks of `seal_and_apply` interact with the Toast Manager.

```typescript
// Inside handleSealAndApply()
try {
  // ... apply logic ...
  window.ToastManager.showToast({
    type: 'success',
    message: 'Configuration applied successfully.',
    // The ToastManager's container must have aria-live="polite"
  });
} catch (error) {
  window.ToastManager.showToast({
    type: 'error',
    message: `Application failed: ${error.message}`
  });
} finally {
  // ... existing UI unlock logic ...
}
```
*Note: Verify that `tachyon-ui/src/components/layout/TachyonToastManager.ts` renders its wrapper with `aria-live="polite"` so these new messages are read aloud.*