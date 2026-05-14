# Implementation Tasks

- [x] **Task 1: Update `utils/a11y.ts`** — Add the Escape key listener and the onClose callback parameter.
- [x] **Task 2: Wire Escape to Existing Modals** — TachyonIAM, TachyonAppShellModalRoot, TachyonBundleConflictModal all pass close logic to trapFocus. BundleConflictModal drops its local copy in favour of the shared utility.
- [x] **Task 3: Instrument TachyonUsersPanel.ts** — role=dialog, aria-modal, aria-labelledby on audit modal; trapFocus called after viewAudit render.
- [x] **Task 4: Announce the Global Loader** — aria-live=polite, aria-atomic=true, sr-only text, aria-hidden on visual spinner in global-apply-loader.
