import { invoke as tauriInvoke } from "@tauri-apps/api/core";

import { connectionStore } from "../stores/connectionStore";
import { ensureMfa } from "./authSudo";
import { translateBackendError } from "./i18n";

const MAX_RETRIES = 5;

export const reconnectDelayMs = (retryCount: number): number => Math.min(1000 * 2 ** retryCount, 30000);

const sleep = (delayMs: number) => new Promise((resolve) => window.setTimeout(resolve, delayMs));
let reconnectLoop: Promise<void> | null = null;

// Allow the store's manualRetry() to kick off a fresh cycle.
window.addEventListener("network:manual-retry", () => {
  reconnectLoop = null;
  startReconnectLoop();
});

type ApplyConfigurationResponse = {
  success: boolean;
  message: string;
  staged: boolean;
  requiresSeal: boolean;
};

type SealApplyOutcome = {
  success: boolean;
  message: string;
  configVersion: number;
};

export async function resilientInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    if (requiresStepUp(command)) {
      await ensureMfa();
    }
    const result = await tauriInvoke<T>(command, args);
    connectionStore.getState().resetRetry();
    connectionStore.getState().setStatus("connected");
    return result;
  } catch (error) {
    connectionStore.getState().setStatus("disconnected");
    startReconnectLoop();
    const raw = error instanceof Error ? error.message : String(error);
    throw new Error(translateBackendError(raw));
  }
}

function requiresStepUp(command: string): boolean {
  return new Set([
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

function startReconnectLoop(): void {
  if (reconnectLoop) {
    return;
  }
  reconnectLoop = (async () => {
    for (let attempt = 1; attempt <= MAX_RETRIES; attempt++) {
      connectionStore.getState().setReconnectionAttempt(attempt, MAX_RETRIES);
      await sleep(reconnectDelayMs(attempt - 1));
      try {
        await tauriInvoke("get_engine_status");
        connectionStore.getState().resetRetry();
        connectionStore.getState().setStatus("connected");
        reconnectLoop = null;
        return;
      } catch {
        // Continue to next attempt.
      }
    }
    // All attempts exhausted: enter terminal disconnected state.
    // Leave attempt === MAX_RETRIES so the UI can distinguish this from
    // a transient disconnect that hasn't started retrying yet.
    connectionStore.getState().setStatus("disconnected");
    reconnectLoop = null;
  })();
}
