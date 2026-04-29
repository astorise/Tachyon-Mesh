import { invoke as tauriInvoke } from "@tauri-apps/api/core";

import { connectionStore } from "../stores/connectionStore";

export const reconnectDelayMs = (retryCount: number): number => Math.min(1000 * 2 ** retryCount, 30000);

const sleep = (delayMs: number) => new Promise((resolve) => window.setTimeout(resolve, delayMs));
let reconnectLoop: Promise<void> | null = null;

export async function resilientInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    const result = await tauriInvoke<T>(command, args);
    connectionStore.getState().resetRetry();
    connectionStore.getState().setStatus("connected");
    return result;
  } catch (error) {
    connectionStore.getState().setStatus("disconnected");
    startReconnectLoop();
    throw error;
  }
}

function startReconnectLoop(): void {
  if (reconnectLoop) {
    return;
  }
  reconnectLoop = (async () => {
    while (connectionStore.getState().status !== "connected") {
      const retryCount = connectionStore.getState().retryCount;
      connectionStore.getState().setStatus("reconnecting");
      console.info(`tachyon-ui reconnect attempt ${retryCount + 1}; waiting ${reconnectDelayMs(retryCount)}ms`);
      await sleep(reconnectDelayMs(retryCount));
      try {
        await tauriInvoke("get_engine_status");
        connectionStore.getState().resetRetry();
        connectionStore.getState().setStatus("connected");
      } catch {
        connectionStore.getState().incrementRetry();
        connectionStore.getState().setStatus("disconnected");
      }
    }
    reconnectLoop = null;
  })();
}
