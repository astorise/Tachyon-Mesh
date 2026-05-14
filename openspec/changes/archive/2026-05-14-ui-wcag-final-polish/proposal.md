# Proposal: WCAG AA Final Polish

## Context
The latest usability audit confirmed that our baseline accessibility (focus traps, semantic landmarks, global loaders) is solid. However, it identified three localized regressions that prevent the application from achieving full WCAG AA compliance.

## Problem
1. **No Escape Hatch:** The current `trapFocus` utility in `a11y.ts` traps the user inside a modal but does not provide a standard keyboard mechanism (`Escape` key) to close it.
2. **Orphaned Modal:** The `TachyonUsersPanel.ts` audit modal was missed during the previous A11y pass. It lacks `role="dialog"`, `aria-modal`, and focus trapping, breaking the consistency established by `TachyonIAM` and `TachyonAppShellModalRoot`.
3. **Silent Loader:** The global deploy loader visually indicates that the system is busy, but lacks `aria-live="polite"`, leaving screen reader users unaware that a background process is running.

## Proposed Solution
1. Enhance `utils/a11y.ts` to accept an `onEscape` callback and bind it to the `keydown` event.
2. Retrofit the `TachyonUsersPanel` modal with the exact same A11y attributes and focus trap used elsewhere.
3. Add `aria-live="polite"` to the global apply loader in `TachyonAppShell.ts`.

## Impact
- Closes the final P0 usability gaps.
- Guarantees a fully accessible, predictable, and WCAG AA compliant interface for operators relying on assistive technologies.