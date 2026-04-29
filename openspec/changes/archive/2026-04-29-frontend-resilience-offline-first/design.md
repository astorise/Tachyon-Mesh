# Design: Connection Manager & Optimistic UI

## 1. State Management (`tachyon-ui/src/stores/connectionStore.ts`)
Create a Zustand store to manage the global connection lifecycle.

```typescript
type ConnectionState = 'connected' | 'disconnected' | 'reconnecting';

interface ConnectionStore {
  status: ConnectionState;
  retryCount: number;
  setStatus: (status: ConnectionState) => void;
  incrementRetry: () => void;
  resetRetry: () => void;
}
```

## 2. API Interceptor / WebSocket Wrapper (`tachyon-ui/src/api/client.ts`)
Wrap the existing Tauri IPC/HTTP calls with a resiliency layer:
- Catch `NetworkError` or `Timeout`.
- Trigger `setStatus('disconnected')`.
- Start the reconnection loop: `delay = Math.min(1000 * (2 ** retryCount), 30000)`.
- Re-poll essential data (like `core-host` status) once reconnected.

## 3. Global UI Indicator (`tachyon-ui/src/components/layout/NetworkStatus.tsx`)
A small UI component placed in the top navigation bar or bottom corner.
- **Green Dot:** Connected.
- **Orange/Yellow Spinning Icon:** Reconnecting (Attempt X).
- **Red Icon:** Disconnected / Offline.

## 4. Optimistic UI Updates (Example: Resource Catalog)
When a user deletes a resource alias in the UI:
1. Immediately remove the item from the local Zustand array.
2. Send the delete request to the backend.
3. If the request fails (after retries), pop a toast notification "Failed to delete" and revert the Zustand state to restore the item.