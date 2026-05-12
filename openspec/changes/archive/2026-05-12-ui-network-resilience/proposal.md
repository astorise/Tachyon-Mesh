# Proposal: Bounded Network Reconnection & Fallback State

## Context
The recent usability audit identified a P0 issue in Tachyon-UI's network layer (`src/utils/network.ts`). The application uses an exponential backoff strategy for reconnections but fails to define a maximum retry limit (`Math.min(1000*2**n, 30000)` runs indefinitely).

## Problem
When the underlying cluster or `core-host` is unreachable, the UI enters a silent infinite loop of network requests. This leads to resource exhaustion in the Tauri WebView, lack of user feedback, and an inability for the user to manually intervene or understand the system's state.

## Proposed Solution
1. **Cap Reconnection Attempts:** Limit the exponential backoff to a fixed number of attempts (e.g., 5 or 10).
2. **Explicit Connection States:** Update `connectionStore.ts` to support distinct states: `connecting`, `connected`, `reconnecting`, and `disconnected` (terminal).
3. **User Feedback & Manual Retry:** Expose the current retry count via the store and display it in `NetworkStatus.ts`. Once the limit is reached, show a clear "Cluster Unreachable" state with a manual "Retry Connection" button.

## Impact
- **Stability:** Prevents silent background freezes.
- **Usability:** Users immediately understand when the cluster is down and can decide when to attempt a reconnection.