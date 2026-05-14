# Design: accessibility-and-loaders

## Task 1 — `tachyon-ui/src/utils/a11y.ts`

`trapFocus(element)` traps keyboard navigation inside a container:
- Queries all focusable children (links, buttons, inputs, selects, `[tabindex]`) excluding hidden and off-screen elements.
- On `keydown Tab`: when at the last focusable element, wraps to the first; on `Shift+Tab` when at the first, wraps to the last.
- Immediately moves focus to the first focusable child.
- Returns a cleanup function that removes the `keydown` listener.

## Task 2 — `TachyonIAM.ts`

- `#iam-panel` gains `role="dialog"`, `aria-modal="true"`, and `aria-labelledby="iam-dialog-title"`.
- The title `<h2>` gains `id="iam-dialog-title"` so the label resolves correctly.
- `connectedCallback()` calls `trapFocus(panel)` immediately after rendering and before the GSAP entrance animation, so focus lands inside the dialog as soon as it appears.

## Task 3 — `TachyonAppShellModalRoot.ts`

`openConflictModal()` now:
1. Sets `role="dialog"` and `aria-modal="true"` on the `<tachyon-bundle-conflict-modal>` element.
2. Calls `modal.open(conflicts)` as before.
3. Calls `trapFocus(modal)` to cycle focus within the conflict dialog.

## Task 4 — `TachyonAppShell.ts` global deploy loader

Two new private helpers:
- `showApplyLoader()` — sets `aria-busy="true"` on `#main-content`, adds `pointer-events-none opacity-50`, creates `#global-apply-loader` div (an absolute overlay with a cyan `animate-spin` ring and "Applying…" label, `role="status"`) and appends it to `#main-content`.
- `hideApplyLoader()` — removes `aria-busy`, removes the loader element, restores pointer events and opacity.

`sealAndApply()` calls `showApplyLoader()` immediately after the initial render and `hideApplyLoader()` in the `finally` block before clearing `applyingSeal`, so the spinner is always removed regardless of success or error.
