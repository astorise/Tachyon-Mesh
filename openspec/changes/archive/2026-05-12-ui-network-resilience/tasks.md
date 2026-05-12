# Implementation Tasks

- [x] **Task 1: Update Store Interface**
  - Modify `src/stores/connectionStore.ts` to include `attempt`, `maxAttempts`, and transition states (`reconnecting`, `disconnected`).

- [x] **Task 2: Implement Bounded Backoff**
  - Refactor `src/utils/network.ts` (specifically `resilientInvoke` and connection listeners) to stop after `MAX_RETRIES` (suggested: 5).
  - Dispatch state updates to `connectionStore` on each failed attempt.

- [x] **Task 3: UI Component Update**
  - Modify `src/components/NetworkStatus.ts` (or create a global banner in `TachyonAppShell`) to consume the new store values.
  - Add the visual counter (e.g., "3/5").
  - Add the manual retry button visible only when state is `disconnected`.

- [x] **Task 4: Testing**
  - Simulate a cluster shutdown while the UI is running.
  - Verify the UI attempts exactly 5 reconnections with increasing delays.
  - Verify the UI stops polling and displays the terminal "Disconnected" state.
  - Click the manual retry button and ensure a new cycle of 5 attempts begins.