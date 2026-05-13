# Implementation Tasks

- [x] **Task 1: Skeleton CSS Utilities**
  - Update `tachyon-ui/src/style.css` (or `shared-sheets.ts` depending on how styles are injected into the shadow DOM) with `.skeleton-pulse`, `.skeleton-text`, and `.skeleton-block`.

- [x] **Task 2: Update `TachyonConfigDashboard`**
  - Implement `withLoadingState` wrapper in `tachyon-ui/src/components/base/TachyonConfigDashboard.ts`.
  - Add error catching logic inside the wrapper that connects to the Toast Manager.

- [x] **Task 3: Refactor Domain Panels**
  - Identify heavy fetch operations in `TachyonTopologyPanel.ts`, `TachyonOverviewPanel.ts`, and `TachyonHardwarePanel.ts`.
  - Wrap these operations in `this.withLoadingState(() => fetchLogic())`.

- [x] **Task 4: Actionable Toasts**
  - Modify `tachyon-ui/src/components/layout/TachyonToastManager.ts` to accept and render an `action` button.
  - Test it by simulating a failed `get_mesh_graph()` call and clicking the inline "Retry" button within the toast.