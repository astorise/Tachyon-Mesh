# Implementation Tasks

- [x] **Task 1: Global Focus States**
  - Update `tachyon-ui/src/style.css` to enforce `focus-visible` styles using Tailwind's ring utilities.

- [x] **Task 2: Semantic HTML in Shell**
  - Refactor the render method of `TachyonAppShell.ts`.
  - Introduce `<nav>`, `<main>`, and `<header>` tags with appropriate `aria-label` attributes.

- [x] **Task 3: IAM Form Accessibility**
  - Audit `TachyonIAM.ts` and `TachyonMfaPrompt.ts`.
  - Wrap all `<input>` elements with proper `<label for="[id]">` tags (using `.sr-only` if visual design dictates hidden labels).

- [x] **Task 4: ARIA Live Regions & Modals**
  - Add `aria-live="polite"` to dynamic metric display areas.
  - Add `role="dialog"` and `aria-modal="true"` to `TachyonBundleConflictModal.ts`.
  - (Optional but recommended) Implement a simple focus trap utility for active modals so keyboard navigation cannot escape the dialog while it is open.