import { invoke as tauriInvoke } from "@tauri-apps/api/core";

import { connectionStore } from "../stores/connectionStore";

export const reconnectDelayMs = (retryCount: number): number => Math.min(1000 * 2 ** retryCount, 30000);

const sleep = (delayMs: number) => new Promise((resolve) => window.setTimeout(resolve, delayMs));

export async function resilientInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    const result = await tauriInvoke<T>(command, args);
    connectionStore.getState().resetRetry();
    connectionStore.getState().setStatus("connected");
    return result;
  } catch (error) {
    const retryCount = connectionStore.getState().retryCount;
    connectionStore.getState().setStatus("disconnected");
    connectionStore.getState().incrementRetry();
    connectionStore.getState().setStatus("reconnecting");
    await sleep(reconnectDelayMs(retryCount));
    throw error;
  }
}
