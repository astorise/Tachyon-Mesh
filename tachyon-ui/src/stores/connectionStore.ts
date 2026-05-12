import { createStore } from "zustand/vanilla";

const MFA_TS_KEY = "tachyon_mfa_timestamp";

export type ConnectionState = "connected" | "disconnected" | "reconnecting";

type ConnectionStore = {
  status: ConnectionState;
  retryCount: number;
  lastMfaTimestamp: number;
  /** Current reconnection attempt number (0 = not retrying or not yet started). */
  attempt: number;
  /** Maximum attempts for the current cycle (0 = cycle limit not yet set). */
  maxAttempts: number;
  setStatus: (status: ConnectionState) => void;
  incrementRetry: () => void;
  resetRetry: () => void;
  setLastMfaTimestamp: (timestamp: number) => void;
  /** Called by the reconnect loop on each failed attempt. Sets status to
   * `"reconnecting"` and records the current attempt index and cycle limit. */
  setReconnectionAttempt: (attempt: number, max: number) => void;
  /** Resets the attempt counter and dispatches `"network:manual-retry"` so the
   * network layer starts a fresh bounded retry cycle. */
  manualRetry: () => void;
};

function readLastMfaTimestamp(): number {
  try {
    const timestamp = Number(window.localStorage.getItem(MFA_TS_KEY) ?? 0);
    return Number.isFinite(timestamp) && timestamp > 0 ? timestamp : 0;
  } catch {
    return 0;
  }
}

function persistLastMfaTimestamp(timestamp: number): void {
  try {
    if (timestamp > 0) {
      window.localStorage.setItem(MFA_TS_KEY, String(timestamp));
    } else {
      window.localStorage.removeItem(MFA_TS_KEY);
    }
  } catch {
    // Storage is best-effort; the in-memory value still gates this session.
  }
}

export const connectionStore = createStore<ConnectionStore>((set) => ({
  status: "connected",
  retryCount: 0,
  lastMfaTimestamp: readLastMfaTimestamp(),
  attempt: 0,
  maxAttempts: 0,
  setStatus: (status) => set({ status }),
  incrementRetry: () => set((state) => ({ retryCount: state.retryCount + 1 })),
  resetRetry: () => set({ retryCount: 0, attempt: 0 }),
  setLastMfaTimestamp: (lastMfaTimestamp) => {
    persistLastMfaTimestamp(lastMfaTimestamp);
    set({ lastMfaTimestamp });
  },
  setReconnectionAttempt: (attempt, maxAttempts) =>
    set({ status: "reconnecting", attempt, maxAttempts }),
  manualRetry: () => {
    set({ attempt: 0, status: "disconnected" });
    window.dispatchEvent(new CustomEvent("network:manual-retry"));
  },
}));

export const useConnectionStore = connectionStore;
