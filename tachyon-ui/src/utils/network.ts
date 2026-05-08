import { invoke as tauriInvoke } from "@tauri-apps/api/core";

import { connectionStore } from "../stores/connectionStore";
import { ensureMfa } from "./authSudo";

export const reconnectDelayMs = (retryCount: number): number => Math.min(1000 * 2 ** retryCount, 30000);

const sleep = (delayMs: number) => new Promise((resolve) => window.setTimeout(resolve, delayMs));
let reconnectLoop: Promise<void> | null = null;

export async function resilientInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    if (requiresStepUp(command)) {
      await ensureMfa();
    }
    const result = await tauriInvoke<T>(command, args);
    if (command === "apply_configuration" && isStagedConfiguration(result)) {
      window.dispatchEvent(new CustomEvent("config:staged", { detail: result }));
    }
    connectionStore.getState().resetRetry();
    connectionStore.getState().setStatus("connected");
    return result;
  } catch (error) {
    connectionStore.getState().setStatus("disconnected");
    startReconnectLoop();
    throw error;
  }
}

function requiresStepUp(command: string): boolean {
  return new Set([
    "apply_configuration",
    "seal_and_apply_manifest",
    "save_resource",
    "delete_resource",
    "push_asset",
    "push_large_model",
    "generate_operator_invite",
    "generate_pat",
    "iam_regen_mfa",
    "generate_recovery_codes",
    "regenerate_account_security",
  ]).has(command);
}

function isStagedConfiguration(result: unknown): result is { requiresSeal: boolean; staged: boolean } {
  return (
    typeof result === "object" &&
    result !== null &&
    (result as { requiresSeal?: unknown }).requiresSeal === true &&
    (result as { staged?: unknown }).staged === true
  );
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
