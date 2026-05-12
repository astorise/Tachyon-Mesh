import { createStore } from "zustand/vanilla";

const MFA_TS_KEY = "tachyon_mfa_timestamp";

export type ConnectionState = "connected" | "disconnected" | "reconnecting";

type ConnectionStore = {
  status: ConnectionState;
  retryCount: number;
  lastMfaTimestamp: number;
  setStatus: (status: ConnectionState) => void;
  incrementRetry: () => void;
  resetRetry: () => void;
  setLastMfaTimestamp: (timestamp: number) => void;
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
  setStatus: (status) => set({ status }),
  incrementRetry: () => set((state) => ({ retryCount: state.retryCount + 1 })),
  resetRetry: () => set({ retryCount: 0 }),
  setLastMfaTimestamp: (lastMfaTimestamp) => {
    persistLastMfaTimestamp(lastMfaTimestamp);
    set({ lastMfaTimestamp });
  },
}));

export const useConnectionStore = connectionStore;
