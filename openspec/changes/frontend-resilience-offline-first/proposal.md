# Proposal: Frontend Resilience and Offline-First UX

## Context
Tachyon Mesh operates in Edge environments where network stability is inherently unreliable. Currently, if `tachyon-ui` (Tachyon Studio) loses its IPC/WebSocket connection to the `core-host`, the UI fails hard: data fetches hang, mutations trigger generic errors, and the user is left without clear feedback. When the connection drops, aggressive immediate reconnections can also cause a thundering herd problem if multiple clients try to reconnect to a rebooting router simultaneously.

## Proposed Solution
We will implement an **Offline-First & Resilient Connection Architecture**:
1. **Connection State Machine:** Introduce a global `useConnectionStore` in Zustand to track `CONNECTED`, `DISCONNECTED`, and `RECONNECTING` states.
2. **Exponential Backoff:** If the connection drops, the UI will attempt to reconnect using an exponentially increasing delay (e.g., 1s, 2s, 4s, 8s, up to a max of 30s) to relieve pressure on the host.
3. **Visual Feedback:** A non-intrusive global banner (or status indicator) will notify the administrator when the dashboard is operating in "Offline Mode".
4. **Optimistic Updates:** For non-critical configuration changes, the UI will update its local Zustand state instantly, providing a snappy UX, and sync with the backend once the connection is restored (or rollback on final failure).

## Objectives
- Prevent UI freezes during network partitions.
- Protect the `core-host` from reconnect storms via Exponential Backoff.
- Provide clear, professional feedback to the user regarding system health.