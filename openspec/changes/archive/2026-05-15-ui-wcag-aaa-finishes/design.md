# Design: WCAG AAA Finishes

## What Was Built

Three targeted accessibility improvements elevating the UI to WCAG 2.4.3 AAA compliance.

### Task 1 — Focus Restoration in `trapFocus` (`utils/a11y.ts`)
- Captures `document.activeElement` as `previousFocus` before trapping starts.
- The returned cleanup function now calls `previousFocus?.focus()` after removing the keydown listener, so keyboard users land back on the trigger element (e.g. the "Seal & Apply" button) when a modal closes.
- Added a JSDoc warning block explicitly documenting that `trapFocus` must **not** be combined with native `<dialog>` elements, which have their own browser-managed focus trap.

### Task 2 — Global Loader Teardown Toasts (`TachyonAppShell.ts`)
- Verified that all code paths inside `sealAndApply` dispatch `app:notify` events (success, conflict-error, fallback-success, fallback-error) **before** `finally` runs.
- The `finally` block calls `hideApplyLoader()` after the toast has already been dispatched, preserving the correct announcement order: toast (aria-live) first, loader teardown second.
- No structural change was required; the ordering was already correct. The missing piece was the toast container's `aria-live` attribute (see Task 3).

### Task 3 — Toast ARIA (`TachyonToastManager.ts`)
- Added `role="status" aria-live="polite" aria-atomic="false"` to the `#toast-container` div.
- `aria-atomic="false"` ensures each individual toast appended to the container is announced independently rather than re-announcing the whole list.
- This is the mechanism that makes Task 2's notifications audible to screen readers: the toast manager was already receiving events and rendering content, but had no live region to trigger announcements.

## Files Changed
- `tachyon-ui/src/utils/a11y.ts`
- `tachyon-ui/src/components/layout/TachyonToastManager.ts`
