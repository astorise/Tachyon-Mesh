import { createStore } from "zustand/vanilla";

export type ConnectionState = "connected" | "disconnected" | "reconnecting";

type ConnectionStore = {
  status: ConnectionState;
  retryCount: number;
  setStatus: (status: ConnectionState) => void;
  incrementRetry: () => void;
  resetRetry: () => void;
};

export const connectionStore = createStore<ConnectionStore>((set) => ({
  status: "connected",
  retryCount: 0,
  setStatus: (status) => set({ status }),
  incrementRetry: () => set((state) => ({ retryCount: state.retryCount + 1 })),
  resetRetry: () => set({ retryCount: 0 }),
}));

export const useConnectionStore = connectionStore;
