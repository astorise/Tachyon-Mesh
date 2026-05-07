# Tasks: Toast Manager

- [x] Create `src/components/layout/TachyonToastManager.ts`.
- [x] Add `<tachyon-toast-manager></tachyon-toast-manager>` to `index.html`, right after the `<tachyon-app-shell>` tag.
- [x] Update the base class `TachyonConfigDashboard.ts` (from Change 1). Modify its `showFeedback` method so that in addition to local DOM updates, it also dispatches the `app:notify` CustomEvent, guaranteeing the user always sees the result.
