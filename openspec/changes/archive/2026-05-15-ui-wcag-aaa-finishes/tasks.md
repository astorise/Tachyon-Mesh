# Implementation Tasks

- [x] **Task 1: Enhance `trapFocus` Utility**
  - Open `tachyon-ui/src/utils/a11y.ts`.
  - Add the JSDoc warning about native `<dialog>` elements.
  - Implement `previousFocus` capture and add `.focus()` restoration to the returned cleanup function.

- [x] **Task 2: Global Loader Teardown Toasts**
  - Open `tachyon-ui/src/components/layout/TachyonAppShell.ts`.
  - In `handleSealAndApply`, ensure a success or error Toast is triggered immediately after the `apply` network call completes, using the existing Toast Manager.

- [x] **Task 3: Verify Toast ARIA**
  - Open `tachyon-ui/src/components/layout/TachyonToastManager.ts`.
  - Verify that the main container holding the toasts (or the individual toast elements) possesses the `aria-live="polite"` or `role="status"` attribute to guarantee the injected messages are announced.