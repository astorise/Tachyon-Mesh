# Proposal: UI P0 Accessibility & Global Deploy Loader

## Context
The post-Codex usability audit identified two remaining P0 issues in the UI. While the foundational WCAG work (focus rings, semantic HTML) was merged successfully, modal overlays remain non-compliant, and the most critical user action (`seal_and_apply`) lacks appropriate visual feedback.

## Problem
1. **Accessibility Escape (WCAG):** The authentication modal (`TachyonIAM.ts`) and the general modal root (`TachyonAppShellModalRoot.ts`) use `fixed z-[100]` positioning but lack `aria-modal="true"` and focus trapping. A screen reader or keyboard user can easily "tab out" of the modal and interact with the invisible background.
2. **Ghosting during Deployment:** When a user clicks "Apply" on a manifest, the button text changes, but the main interface remains interactive and unshaded. Because the `seal_and_apply` cryptographic process can take several seconds, users often believe the app has frozen.

## Proposed Solution
1. **Strict Modal A11y:** Inject `role="dialog"` and `aria-modal="true"` into the modal containers. Implement a lightweight focus trap utility that restricts `Tab` and `Shift+Tab` navigation to the modal's DOM tree while it is open.
2. **Global Busy State:** Introduce a translucent overlay with a spinner and `aria-busy="true"` on the `<main>` element during the `seal_and_apply` execution in `TachyonAppShell.ts`.

## Impact
- **Compliance:** Achieves full WCAG baseline compliance for dialogs.
- **UX Confidence:** Users receive unambiguous feedback that a heavy cryptographic/network operation is underway.