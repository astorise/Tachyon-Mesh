# Implementation Tasks

## Phase 1: Connection Store & Core Logic
- [x] Create `connectionStore.ts` using Zustand.
- [x] Implement the Exponential Backoff utility function in `tachyon-ui/src/utils/network.ts`.
- [x] Wrap the main data-fetching hook (or Tauri IPC wrapper) to catch connection failures and update the store.

## Phase 2: Visual Feedback
- [x] Create the `NetworkStatus.tsx` component with SVG icons (using `lucide-react` or Tailwind SVGs).
- [x] Integrate `NetworkStatus.tsx` into the main `TachyonStudio` layout (e.g., Header or Sidebar footer).
- [x] Add smooth GSAP color transitions (Green -> Orange -> Red) based on the state.

## Phase 3: Optimistic Mutations
- [x] Refactor the `ResourceCatalogView` (from our previous change) to use Optimistic Updates when adding/deleting a resource.
- [x] Ensure a rollback mechanism exists in the `catch` block of the mutation.

## Phase 4: Validation
- [x] **Test Disconnect:** Run the UI, manually kill the `core-host` process. Verify the UI switches to "Reconnecting" and does not freeze.
- [x] **Test Backoff:** Watch the network tab / console logs to verify the delay between reconnection attempts doubles each time.
- [x] **Test Reconnect:** Restart the `core-host`. Verify the UI automatically detects the connection, turns green, and fetches fresh data.
