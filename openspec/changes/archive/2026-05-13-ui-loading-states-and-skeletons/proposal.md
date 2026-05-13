# Proposal: UI Loading States & Actionable Error Toasts

## Context
The recent P1 usability audit highlighted a significant UX deficiency in `Tachyon-UI`: the absence of visual loading states. Currently, when data is fetching (e.g., `get_mesh_graph()`), the UI relies on static i18n strings like `"Loading mesh telemetry…"`. To the user, the application often feels frozen. Furthermore, errors are not actionable.

## Problem
1. **Perceived Performance:** Without skeleton screens or spinners, perceived latency is much higher than actual latency.
2. **Code Duplication:** Loading logic is manually handled (or ignored) across the 11 different domain panels (`TachyonIdentityPanel`, `TachyonTopologyPanel`, etc.).
3. **Dead Ends:** When an operation fails, the user gets a static error rather than a clear path to recovery (e.g., a "Retry" button).

## Proposed Solution
1. **Skeletons & Spinners (Tailwind v4):** Implement reusable CSS-based skeleton animations and SVG spinners in our global styles.
2. **`WithLoadingState` Mixin:** Extract a standard loading wrapper or mixin for `TachyonConfigDashboard.ts` to automatically handle `isLoading` and `error` states for all inheriting domain panels.
3. **Actionable Toasts:** Upgrade `TachyonToastManager.ts` to accept callback functions, allowing toasts to render inline `<button>` elements for "Retry" or "Rollback" actions.

## Impact
- **UX:** Smooth, predictable interface transitions that respect modern desktop app standards.
- **DX:** Domain panels no longer need to manually toggle CSS classes for loading states; they simply wrap their async `connectedCallback` logic in the mixin.