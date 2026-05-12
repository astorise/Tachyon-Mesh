# Technical Specification: Bounded Reconnection

## 1. Network Utility (`src/utils/network.ts`)
Modify the `resilientInvoke` and core connection logic to enforce a maximum retry count.

```typescript
const MAX_RETRIES = 5;
const BASE_DELAY_MS = 1000;
const MAX_DELAY_MS = 30000;

export async function resilientInvoke<T>(cmd: string, args?: any): Promise<T> {
  let attempt = 0;
  
  while (attempt < MAX_RETRIES) {
    try {
      return await tauriInvoke<T>(cmd, args);
    } catch (err) {
      attempt++;
      connectionStore.getState().setReconnectionAttempt(attempt, MAX_RETRIES);
      
      if (attempt >= MAX_RETRIES) {
        connectionStore.getState().setConnectionState('disconnected');
        throw new Error(`Command ${cmd} failed after ${MAX_RETRIES} attempts.`);
      }
      
      const delay = Math.min(BASE_DELAY_MS * 2 ** attempt, MAX_DELAY_MS);
      await new Promise(res => setTimeout(res, delay));
    }
  }
  throw new Error("Unreachable"); // Should be caught by the loop logic
}
```

## 2. Store Updates (`src/stores/connectionStore.ts`)
Extend the Zustand store to hold retry telemetry.

```typescript
interface ConnectionState {
  status: 'connecting' | 'connected' | 'reconnecting' | 'disconnected';
  attempt: number;
  maxAttempts: number;
  setConnectionState: (status: ConnectionState['status']) => void;
  setReconnectionAttempt: (attempt: number, max: number) => void;
  manualRetry: () => Promise<void>;
}
```

## 3. UI Feedback (`src/components/NetworkStatus.ts`)
Implement the visual representation of these states.

- **State: `reconnecting`**: Show an amber banner/toast or a spinner: *"Reconnexion au cluster (Essai ${attempt}/${maxAttempts})..."*
- **State: `disconnected`**: Show a red banner/toast: *"Cluster injoignable. Vérifiez que core-host tourne."* with a `<button>` bound to `connectionStore.getState().manualRetry()`.