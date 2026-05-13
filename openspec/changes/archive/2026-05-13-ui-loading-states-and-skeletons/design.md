# Design: Loading States & Skeleton UX Foundation

## Approach

Four additive changes. No existing behaviour is broken — all are opt-in additions to the existing shadow-DOM component hierarchy.

### 1. Skeleton CSS (`style.css`)

Three utility classes are added in a `@layer utilities` block with a custom `@keyframes skeleton-shimmer` animation. The shimmer uses a sliding `background-position` gradient (dark slate tones) that fits the existing slate-950 design language. `@apply` is intentionally avoided since Tailwind v4 discourages `@apply` for custom utilities; the classes are written as plain CSS.

### 2. `withLoadingState` + `handlePanelError` (`TachyonConfigDashboard.ts`)

`withLoadingState<T>(task, containerSelector?)` is a protected async method on the shared base class. It:
1. Finds the target container via `this.root.querySelector(containerSelector)` (shadow root, not light DOM).
2. Replaces its `innerHTML` with four shimmer skeleton divs — safe because the template is entirely static.
3. Awaits `task()`.
4. On success: returns the result; the caller's normal `render()` overwrites the skeleton.
5. On failure: delegates to `handlePanelError(error, task)`.

`handlePanelError` dispatches a `"toast"` custom event carrying a `ToastDetail` with `type: "error"` and an optional `action: { label: "Retry", onClick }` that re-invokes `withLoadingState`. This connects the base class to the toast layer without a direct import.

### 3. Panel integration

| Panel | Change |
|---|---|
| `TachyonTopologyPanel` | `loadLiveTopology()` body wrapped in `withLoadingState(async () => { … })` |
| `TachyonOverviewPanel` | The try/catch block in `connectedCallback` replaced with `withLoadingState(async () => { … })` |
| `TachyonHardwarePanel` | New `get_hardware_status` Tauri invoke added; `connectedCallback` changed to `async`; live RAM/accelerator badge rendered in the header |

### 4. Actionable toasts (`TachyonToastManager.ts`)

`ToastDetail` gains an optional `action?: { label: string; onClick: () => void }` field. When present, an inline button is appended to the toast element (before the dismiss `×`) using `document.createElement`. The button calls `action.onClick()` and then `dismissToast`. Toasts with an action have an 8-second TTL instead of 4 seconds. The `ToastKind` type is extended to include `"warning"` and `"info"` with matching colour tones.

## Trade-offs

| Decision | Chosen | Rejected | Reason |
|---|---|---|---|
| Skeleton target | `this.root.querySelector` | `this.querySelector` | These are shadow-DOM components; light-DOM query misses all shadow content |
| Skeleton content | Static `innerHTML` | `createElement` nodes | Template is fully static (no user data); safe and simpler to read |
| `@apply` usage | Avoided — plain CSS | `@apply` in `@layer utilities` | Tailwind v4 recommends plain CSS utilities; `@apply` in custom layers is deprecated |
| Toast action TTL | 8 s (vs 4 s default) | Same 4 s | User needs time to read and click the Retry button |
| `HardwarePanel` fetch | Added new `get_hardware_status` call | Left as static form | Panel was the only one of the three without a fetch; adding live data makes the skeleton meaningful |
