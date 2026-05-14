# Implementation Tasks

- [x] **Task 1: Focus Trap Utility**
  - Create `tachyon-ui/src/utils/a11y.ts` and implement the `trapFocus` function.

- [x] **Task 2: Update IAM Modal**
  - Edit `TachyonIAM.ts` (around line 81).
  - Add `role="dialog"` and `aria-modal="true"`.
  - Invoke `trapFocus` on the active step component (`<auth-step-credentials>` or `<auth-step-mfa>`).

- [x] **Task 3: Update Modal Root**
  - Edit `TachyonAppShellModalRoot.ts` (around line 47).
  - Add `role="dialog"` and `aria-modal="true"` to the dynamic backdrop.
  - Invoke `trapFocus` when a new modal is pushed to the stack.

- [x] **Task 4: Global Deploy Loader**
  - Edit `TachyonAppShell.ts` (around lines 327-403).
  - Inject the translucent overlay and CSS spinner during the `seal_and_apply` execution, ensuring `aria-busy="true"` is toggled on the `<main>` container.
