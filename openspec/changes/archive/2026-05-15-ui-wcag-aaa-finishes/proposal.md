# Proposal: WCAG AAA Finishes

## Context
Following the completion of the P0 WCAG AA accessibility pass, a secondary sub-agent audit identified three highly specific P2 usability improvements that elevate the interface to a AAA standard for assistive technologies.

## Problem
1. **Silent Async Completion:** The global apply loader (`TachyonAppShell.ts`) announces its start via `aria-live`, but its teardown is silent. Screen reader users are left wondering if the "Applying configuration..." step succeeded, failed, or just vanished.
2. **Focus Amnesia (WCAG 2.4.3):** When a modal is closed, the focus is dropped to the document `<body>` rather than being restored to the button or element that originally triggered the modal. This forces keyboard users to restart their navigation from the top of the page.
3. **Documentation Ambiguity:** `TachyonMfaPrompt` uses the native HTML5 `<dialog>` element, which handles focus trapping natively. Our custom `trapFocus` utility does not explicitly document that it should *not* be combined with native dialogs, risking future conflicting behavior.

## Proposed Solution
1. **Automatic Focus Restoration:** Upgrade `trapFocus` in `utils/a11y.ts` to automatically capture `document.activeElement` on initialization and restore focus to it during teardown. Add JSDoc to clarify the native `<dialog>` exception.
2. **Explicit Teardown Announcement:** Hook the end of the `seal_and_apply` process in `TachyonAppShell.ts` to the `TachyonToastManager`, ensuring success or failure messages are announced via the toast's existing `aria-live` region.

## Impact
- **Inclusivity:** Flawless keyboard navigation and complete lifecycle transparency for screen readers.
- **Developer Experience:** Clear boundaries for when to use custom A11y utilities versus native browser features.